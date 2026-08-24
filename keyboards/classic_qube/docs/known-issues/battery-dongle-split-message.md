# Известная проблема: батарея `??` на Qube dongle (RMK split)

**Статус:** обойдено в этом репозитории (в upstream RMK не исправлено)
**Железо:** Ergohaven OP36 + Qube dongle (nRF52840)
**Симптомы:** клавиши работают, половины подключены, на экране статуса батарея вечно `??` / `U?`
**Связь:** на ZMK для тех же плат батарея отображается корректно

---

## Кратко

На dongle уровень батареи «неизвестен» по **двум независимым** причинам:

1. **Разный layout enum `SplitMessage` для postcard**, когда dongle собран **с** фичей `display`, а половины — **без**.
2. **Штатный путь SAADC в RMK** (`battery_adc_pin` в `keyboard.toml`) может **зависнуть** на `calibrate().await` под SoftDevice/MPSL — процент никогда не публикуется.

Железо (пин и делитель) в порядке и совпадает с ZMK.

---

## Ожидаемый путь данных

```
половина (left / right)                      dongle (central + ST7789)
───────────────────────                      ────────────────────────
SAADC P0_31 → % → BatteryStatusEvent
                    ↓
         SplitMessage::BatteryStatus  ── BLE notify ──►  PeripheralBatteryEvent
                                                              ↓
                                                         UI: "87" или "??"
```

### Железо (эталон ZMK)

Из [ergohaven-zmk](https://github.com/ergohaven/ergohaven-zmk), `boards/shields/op36/op36.dtsi`:

```dts
vbatt {
    compatible = "zmk,battery-voltage-divider";
    io-channels = <&adc 7>;              /* AIN7 = P0_31 */
    output-ohms = <1564000>;             /* измеряемое плечо (к GND) */
    full-ohms = <(806000 + 1564000)>;    /* всего = 2370k */
};
```

Те же значения используются здесь (1564 / 2370 в единицах делителя RMK).

---

## Проблема 1 — discriminants postcard у `SplitMessage` (критично)

Сообщения split сериализуются **postcard** по **индексу варианта enum**.

### Без `display` (типичные left/right)

| Индекс | Вариант          |
|-------:|------------------|
| 0      | Key              |
| …      | …                |
| 8      | **BatteryStatus** |

### С `display` (qube через `st7789` / `rmk/display`)

Перед батареей вставляются дополнительные варианты:

| Индекс | Вариант          |
|-------:|------------------|
| 0      | Key              |
| …      | …                |
| 8      | **Wpm**          |
| 9      | Modifier         |
| 10     | SleepState       |
| 11     | **BatteryStatus** |

### Эффект

- Половина кодирует батарею как **индекс 8**.
- Dongle читает индекс 8 как **Wpm** (или падает на десериализации).
- `PeripheralBatteryEvent` не доходит до UI → `Unavailable` → `??`.
- **Клавиши при этом работают**, потому что `Key` = индекс **0** с обеих сторон.

### Диагностический UI (во время отладки)

| На экране | Смысл |
|-----------|--------|
| `--`      | Половина не подключена по split |
| `U?`      | Подключена, но событие батареи не пришло |
| `P?`      | Available, но `level: None` |
| число     | Реальный процент |

`U?` подтверждает: связь есть, обрыв именно на пути батареи (часто на (де)сериализации).

### Обход в этом репозитории

Включить фичу RMK **`display`** на **всех** бинарниках (включая left/right), даже без экрана на половинах:

```toml
# Cargo.toml
rmk = { ..., features = [ ..., "display" ] }
```

Тогда layout `SplitMessage` совпадает с dongle.

### Правильный фикс upstream

В RMK `rmk/src/split/mod.rs` держать `BatteryStatus` **выше** всех вариантов с `#[cfg(feature = "display")]` (или всегда собирать один и тот же список вариантов), чтобы discriminants не зависели от набора feature.

---

## Проблема 2 — stock `battery_adc_pin` зависает / «тихая» батарея

Только toml:

```toml
[[split.peripheral]]
battery_adc_pin = "P0_31"
adc_divider_measured = 1564
adc_divider_total = 2370
```

RMK генерирует `NrfAdc` и обычно делает:

```rust
adc.calibrate().await;  // может зависнуть навсегда под SoftDevice/MPSL
```

Если calibrate не возвращается:

- Matrix / BLE / клавиши продолжают работать.
- Нет сэмплов ADC → нет `BatteryStatusEvent`.
- В stock split нет надёжного battery-heartbeat.
- Dongle навсегда остаётся на `??`.

### Обход в этом репозитории

**Не** задавать `battery_adc_pin` в `keyboard.toml` (нет stock SAADC + calibrate).

Маленький свой reader:

- `src/battery_nrf.rs` — SAADC на `P0_31`, **без calibrate**, sample с timeout, публикация `BatteryStatusEvent` roughly каждые 2 с
- `src/left.rs` / `src/right.rs` — `#[register_processor(event)]`

Повторная отправка нужна, чтобы split-loop успел подписаться и переслать статус на central, даже если первое событие потерялось.

### Правильный фикс upstream

- Не зависать на `calibrate()` на nRF + SoftDevice (skip / harden).
- Re-publish / heartbeat батареи на peripheral, чтобы central не «залипал» после потерянного первого события.

---

## Что оставлено в этом репо (минимум)

| Часть | Роль |
|-------|------|
| `Cargo.toml` → фича `display` у rmk | Тот же layout `SplitMessage`, что у dongle |
| `src/battery_nrf.rs` + register на left/right | Батарея без зависания stock calibrate |
| Нет `battery_adc_pin` на peripheral | Нет double SAADC / stock hang |
| `[event] peripheral_battery.pubs = 2` | Две половины → dongle |

Локальный path на RMK для этого обхода **не патчится**.

---

## Как проверить после прошивки

Прошить **все три**: left, right, `qube`. Подождать ~5–10 с после коннекта.

| Результат | Интерпретация |
|-----------|----------------|
| Реальные % слева/справа | Путь OK |
| Снова `??` | Проверить `display` на halves, наличие custom battery, отсутствие `battery_adc_pin` |
| Нет клавиш / нет связи | Отдельная проблема (пара/storage/не тот бинарник) |

---

## Связанное: полный экран Qube (не батарея)

Полноэкранная отрисовка 280×240 без FB ~134 KB: полосы 280×48 (~27 KB) + multipass
(см. `src/qube_display.rs` — stripe multipass, EasyDMA &lt; 64 KB).

---

## Ссылки

- Документация RMK wireless / split battery: <https://rmk.rs/main/docs/configuration/wireless>
- ZMK OP36 battery: [ergohaven-zmk `op36.dtsi`](https://github.com/ergohaven/ergohaven-zmk/blob/main/boards/shields/op36/op36.dtsi)
- ZMK Qube battery fetch: [ergohaven-zmk-qube](https://github.com/ergohaven/ergohaven-zmk-qube) (`CONFIG_ZMK_SPLIT_BLE_CENTRAL_BATTERY_LEVEL_FETCHING`)
- Тип RMK: `rmk/src/split/mod.rs` → `SplitMessage`
- Codegen SAADC: `rmk-macro/src/codegen/input_device/adc.rs` (`calibrate`)
