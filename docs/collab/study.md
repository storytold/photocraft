# Study: PhotoCraft — модель документа и пригодность к совместному редактированию

> Артефакт Phase 0. Кода фичи не содержит. Все утверждения — из чтения репозитория
> `storytold/photocraft` на `830316b77` (0.23.1). Где число — арифметика или оценка по
> коду, а не замер, это сказано явно: на этой машине проект не собирается (AGENTS.md).

- Проект: `photocraft`
- Upstream: `https://github.com/storytold/photocraft`
- Коммит: `830316b77` (0.23.1), ветка `collab/study` от `main`
- Дата: 2026-11-25
- Автор: `photocraft/envoy` (craft-scout)

## 1. Карта репозитория

Workspace: resolver 3, edition 2024, rust-version 1.95 (`Cargo.toml:1-8`). 25 крейтов в
`crates/`, 4 приложения в `apps/` (`photocraft`, `photocraft-cli`, `photocraft-web`,
`photocraft-web-boot`) + `xtask`. Слои проверяются `cargo xtask layers` по таблице
`xtask/src/layers.rs:35-79`; правило — «ниже L6 нет egui/winit» (`xtask/src/layers.rs:110-116`),
в L0-L5 всё собирается под `wasm32` (`docs/architecture.md:182`).

LOC — измерено мной 2026-11-25: `find … -name '*.rs' -not -path '*/tests/*' -not -path
'*/examples/*' -not -path '*/benches/*' | xargs cat | wc -l`; в скобках — с тестами.
`#[test]` — grep-подсчёт, 15 467 на весь workspace.

| Crate | Слой | Назначение | LOC (src) |
|---|---|---|---|
| `geom` | L0 | точки/аффины, `TILE_SIZE = 256` (`crates/geom/src/lib.rs:13`) | 2 020 |
| `color` | L0 | `PixelFormat`, `ColorMode`, `SampleType`, blend-математика | 1 967 |
| `raster` | L0 | `Surface` из 256² COW-тайлов, spill на диск, `headroom` | 7 916 |
| `doc` | L1 | модель документа: `Document`, `Layer`, `LayerContent`, 87 структур / 46 enum | 6 794 |
| `ops` | L2 | история: полные снапшоты `Arc<Document>` на транзакцию, бюджет памяти | 2 275 |
| `paint` | L2 | кистевой движок, детерминированный rng от `seed` (`crates/paint/src/dynamics.rs:5`) | 14 010 |
| `algo` | L2 | adjustment/filters/selection/inpaint/warp (CPU reference) | 90 023 |
| `text` | L2 | шейпинг/лейаут, растеризация текст-слоя | 20 834 |
| `vector` | L2 | пути, фигуры, coverage (vello_cpu/kurbo) | 6 813 |
| `compose` | L3 | CPU-композитор (референс), «дерево слоёв → пиксели» рект-за-ректом | 28 966 |
| `gpu` | L3 | wgpu-композитор, страницы резидентов, кэш эффектов | 12 548 |
| `format` | L3 | `.pcraft`: manifest.json + `tiles/<blake3>.zst` | 7 764 |
| `io` | L4 | импорт/экспорт, doc ⇄ PSD/ORA/KRA/XCF/PDN/Affinity/RAW | 52 330 |
| `plugins` | L4 | wasm-фильтры (wasmi, ABI v1) | 1 626 |
| `engine-core` | L5 | `Session`, `CommandRegistry`, `DocState`, `cmd/*` (весь командный слой) | 188 451 |
| `engine-paint`, `engine-vector`, `engine` | L5 | командные таблицы инструментов; фасад `Session` | 27k+10k+3k |
| `ui-egui` | L6 | egui-оболочка, панели, инструменты, control-канал | 271 167 |
| `automation` | L6 | MCP + headless JSON-lines сервер (`photocraft-cli serve`), `Headless` | 9 738 |
| `raw`, `tablet`, `ipp`, `oil-round-proof`, `testkit` | standalone/test | RAW-декодер, перьевой ввод, CUPS, тесты | 10k/1.8k/6/34/549 |
| `apps/photocraft` | L7 | desktop (winit+wgpu+ui-egui) | 14 498 |
| `apps/photocraft-cli` | L7 | `run/convert/batch/serve/mcp` | 5 089 |
| `apps/photocraft-web` | L7 | wasm32 + trunk, `?cpu`, `?webgl`, `?free-mb=N` | 2 106 |
| `xtask` | — | layers, wasm, parity, corpus, perf, panel-commands | — |

Итого измерено: **791 111 строк Rust в 2 251 файле src** (907 052 со тестами/examples).
`docs/architecture.md:11` говорит о «~539 000 строках в ~1 310 файлах» — цифра от
2026-10-13 и к этому коммиту устарела.

**Где проходит граница UI.** `Session` живёт в `crates/engine-core/src/lib.rs:193`
(53 поля: документы, `ToolState`, `journal`, `prefs`, jobs, слоты — подсчёт по struct), фасад —
`crates/engine/src/facade.rs:63` (`Session(CoreSession)` + `install()`). Всё ниже L6 не знает
про egui; `ui-egui` зависит от `photocraft-engine` (`crates/ui-egui/Cargo.toml:29`).
Инструменты (pointer-события, оверлеи) — модули `crates/ui-egui/src/tools/*`, отдельного
крейта `photocraft-tools` и трейта `Tool` в коде **нет** (в плане `docs/architecture.md:232-241`
есть, в дереве — нет). Для collab это значит: headless-сервер умеет команды, но не
pointer-жесты; жест должен быть нормализован в команду (`paint.stroke` с точками и `seed`).

## 2. Модель документа

Корень — `Document` (`crates/doc/src/lib.rs:947-1013`), **33 поля**, `Clone + PartialEq`,
без `Serialize`: на диск идёт отдельная serde-модель `format::manifest::DocM`
(`crates/format/src/manifest.rs:31`), конверсия — `crates/format/src/convert.rs` (816 строк).
`Document` — не дерево-арена, а вложенные `Vec`:

- `layers: LayerTree` (`crates/doc/src/lib.rs:956`) — обёртка над `Vec<Layer>` с ленивым
  индексом id (`crates/doc/src/layer_tree.rs:100-110`). **Порядок = индекс в Vec**,
  `children[0]` — нижний слой (`crates/doc/src/lib.rs:1-6`).
- `channels: Vec<AlphaChannel>`, `selection: Option<Surface>`, `quick_mask`, `paths`,
  `work_path`, `guides`, `slices`, `patterns`, `color_table`, `layer_comps`, `timeline`,
  `metadata` (XMP/EXIF, крупные блобы в `Arc<Vec<u8>>`/`Arc<str>`), `icc_profile:
  Option<Arc<Vec<u8>>>`.

`Layer` (`crates/doc/src/lib.rs:691-745`) — **23 поля**, тоже плоская:
`id: LayerId`, `name`, `visible`, `locks`, `blend`, `opacity`, `fill_opacity`, `clipped`,
`mask: Option<LayerMask>`, `vector_mask`, `effects: Effects`, `label`, `content:
LayerContent`, `frame`, `psd_blocks: Vec<([u8;4], Arc<Vec<u8>>)>` (непромоделированные
PSD-блоки — для round-trip), `psd_id`, `fill_cache`, `link_group`, `excluded_channels`,
`blend_if`, `advanced`, `video`, `xmp`.

`LayerContent` (`crates/doc/src/lib.rs:658-666`) — 7 вариантов:
`Raster(Surface)`, `Group(Group)`, `Adjustment(Adjustment)` (17 вариантов,
`crates/doc/src/adjust.rs`), `Fill(Fill)`, `Text(TextLayer)`, `Shape(ShapeLayer)`,
`Smart(SmartObject)`.

Ключевое для collab:

| Часть | Тип | Владение / изменяемость | Годится ли как единица синхронизации |
|---|---|---|---|
| Пиксели слоя/маски | `Surface` (`crates/raster/src/lib.rs:319-325`) | `BTreeMap<TileCoord, Arc<Tile>>`, запись через `Arc::make_mut` (COW), `generation: AtomicU64` на поверхность | да, тайл 256² — натуральная дельта |
| Маска | `LayerMask { surface: Surface, enabled, linked, density, feather }` (`:249-256`) | отдельная поверхность, таргет `"target":"mask"` в paint-командах | да, отдельный от пикселей слой |
| Корректирующий слой | `Adjustment` — чистые параметры | значение в enum | да, LWW-параметры |
| Стиль слоя | `Effects` (10 видов эффектов) | часть `Layer` | да, но это «набор полей», не атом |
| Текст | `TextLayer` (`:448-470`): строки, runs, wrap, `cache: Option<Surface>` | кэш — производный растр **внутри документа** | параметры — да; `cache` — нет |
| Вектор | `ShapeLayer`/`Path`/`VectorMask` (`crates/doc/src/vector.rs`), `cache` | пиксельный кэш тоже в документе | параметры — да; `cache` — нет |
| Смарт-объект | `SmartObject` (`:569-599`): `contents_id`, `source`, `transform`, `smart_filters`, `cache`, `psd_raw`, `filter_mask`, `warp`, `perspective` | `source` — `Embedded{bytes: Arc<Vec<u8>>}` или `Linked{path}` (`:628-631`) | да; байты — по ссылке/хэшу, не в операции |

Важно: `SmartFilter` (`crates/doc/src/lib.rs:635-642`) — это буквально
`{command: String, params: serde_json::Value, blend, opacity, visible}`. В документе уже
есть сериализуемая «операция как команда+параметры», и по ней работают неразрушающие
фильтры. Это самый близкий к протоколу тип, который уже есть.

Производные кэши внутри документа (`TextLayer::cache`, `SmartObject::cache`,
`FillCache`) попадают и в снапшоты, и в `.pcraft` (`ContentM::Text{cache}`,
`ContentM::Smart{cache}`, `LayerM::fill_cache`): два клиента, пересчитавшие кэш на разных
шрифтах/GPU, дают разные пиксели при одинаковых параметрах. Их нельзя мержить как контент —
только пересчитывать/выбирать по правилу «кэш одного писателя».

## 3. Идентификаторы

- `LayerId(pub u64)` — `crates/doc/src/lib.rs:63`; выдаётся `LayerId::fresh()` из
  **процессного** счётчика `static NEXT_ID: AtomicU64 = AtomicU64::new(1)` (`:56-70`).
- Тот же счётчик обслуживает `DocId`, `SmartContentsId`, id истории и т.п.;
  `ensure_ids_above(max)` (`:83`) поднимает его после загрузки файла, чтобы новые id не
  столкнулись с загруженными.
- `LayerId` сериализуется в `.pcraft` как `u64` (`manifest.rs:209`), т.е. **внутри
  документа стабилен и переживает сохранение**; `psd_id` хранит Photoshop'ский `lyid`
  отдельно (`crates/doc/src/lib.rs:721-723`).
- При удалении слоя id не переиспользуется (счётчик только растёт), при вставке
  соседние id не меняются — стабильность при удалении/вставке есть.
- Для сети **не годится как есть**: два процесса независимо начнут с 1 и выдадут
  одинаковые id. Нужен диапазон на актора (`actor_index << 48 | counter`, как в TASK §3)
  либо выдача id сервером при подтверждении вставки.
- Адресация «частей» уже есть в командах: `"target": "pixels"|"mask"|"filterMask"|
  "quickMask"|{"channel": i}` (`crates/engine-paint/src/cmd/table/paint/stroke.rs:5-15`),
  т.е. (layer id, part) — существующая пара, а не изобретение протокола.
- Порядок слоёв адресуется **не** индексом в командах: `layer.moveTo {layer, target,
  position: above|below|into}` (тесты `crates/engine-core/src/cmd/background_cmds.rs:165-185`),
  вставка — `Document::insert_above(relative: Option<LayerId>)` (`crates/doc/src/lib.rs:1335`).
  Индексы/ordinal тоже принимаются (`crates/engine-core/src/cmd/layer_params.rs:34`), но
  id-якорь доступен — это снимает половину структурных конфликтов.

## 4. Команды

- Единая точка мутации: `Session::execute(id: &str, params: Value)`
  (`crates/engine-core/src/session/execute.rs:14`). Внутри — `dispatch` → `CommandSpec::run`.
  Все фронтенды (UI, CLI, control-канал, MCP) идут через неё
  (`crates/engine-core/src/lib.rs:1-6`).
- Реестр: `CommandSpec { id, label, menu, shortcut, params: &'static str, enabled, run,
  journal }` (`crates/engine-core/src/registry.rs:16-29`); таблицы собираются `Install`
  (`:31-55`), порядок — `crates/engine/src/install/command_ids.txt`, **1 018 id** (измерено
  `wc -l`). `docs/architecture.md:17` говорит «~700» — устарело.
- Параметры — не schemars, а **документированная строка** `{"radius":0.1..1000=1}`,
  которую парсит `param_schema` (`crates/engine-core/src/param_schema.rs:1-16`) и по ней
  же проверяет ключи `param_check` (`:1-10`). Строка воспроизводится байт-в-байт тестом.
  Практически: у каждой команды есть машиночитаемая схема значений, но не JSON Schema.
- Валидация: неизвестные ключи отклоняются, кроме universal `coalesce`/`target`
  (`param_schema.rs:27`). Коалесинг (`"coalesce": key`) склеивает серию правок в один шаг
  истории (`session/execute.rs:4-12`) — это готовый механизм «один штрих = одна операция».
- Журнал: `Session.journal: Vec<(String, Value)>` (`crates/engine-core/src/lib.rs:223`) —
  id+params без секретов, неограниченный по длине; используется записью экшенов
  (`crates/engine-core/src/actions_record.rs`) и replay'ем. Это **готовая форма журнала
  команд**, но без лимитов и без инверсий.
- Пиксельная правка тоже команда: `paint.stroke` принимает
  `points:[[x,y,pressure,tiltX,tiltY,rotation,timeMs,wheel],…]`, `brush`, `seed: u64`,
  `target` (`crates/engine-paint/src/cmd/table/paint/stroke.rs:5-15`). В `crates/paint`
  jitter считается хэшем `seed`+индекса дэба (`crates/paint/src/brush.rs:639-640`,
  `dynamics.rs:5`), с комментарием «so strokes replay» — штрих воспроизводим по параметрам.
- Прочие каналы: control-протокол (JSON-lines на 127.0.0.1, challenge-HMAC auth,
  `engine.execute {command, params}` — `docs/control-protocol.md:57,93,97`), MCP
  (`docs/mcp.md`), headless JSON-lines сервер `photocraft-cli serve`
  (`crates/automation/src/rpc.rs:1-20`), плагины-wasm (`docs/plugins.md`).

## 5. Undo

- `crates/ops` — это **только история**, не «операции»: `History { undo: VecDeque<HistoryState>,
  redo, max_states, max_bytes, … }` (`crates/ops/src/lib.rs:52-115`).
- `HistoryState` (`:32-44`) хранит `doc: Arc<Document>` — **полный снапшот документа до
  шага**, плюс `label`, `LayerTarget`, `visibility`, `id`. `History::record(label, before,
  layers)` (`:178`) вызывается из `Session::edit_with`
  (`crates/engine-core/src/session/execute.rs:220,323`) — то есть каждая мутация уже
  проходит через «клонировать документ, применить, положить прежний Arc в историю».
- Стоимость снапшота: `Document::clone` = копия 33 полей + `Vec<Layer>` (23 поля на слой,
  строки/векторы клонируются) + `BTreeMap` тайлов каждой поверхности. Пиксели не
  копируются (COW `Arc`), но **узлы карты и `Layer` клонируются**: для 100 МП RGBA8 это
  1 600 тайлов на слой (`100e6/65536`), 30 слоёв ≈ 48 000 Arc-указателей на снапшот
  (арифметика, не замер).
- Бюджет: `max_states` по умолчанию 50 (`crates/engine-core/src/prefs.rs:870`), байтовый
  лимит = Memory Usage (по умолчанию 8 192 MB, `prefs.rs:728,834-843`), пока включён
  scratch-диск — лимит снимается (тайлы уходят на диск). Учёт байтов — `crates/ops/src/tiles.rs`
  (считает уникальные `Arc`-тайлы и байты встроенных смарт-объектов, `:22-40`).
- **Per-user undo из этого не строится.** История глобальная и на документ; состояние —
  «документ до шага», включая чужие правки. `undo()` вернёт `Arc<Document>`, в котором
  откатятся и правки другого актора. Серверные снапшоты годятся для восстановления и
  resync, но не для undo участника. Для per-user нужно: (а) журнал операций с инверсиями
  на актора, (б) undo = новая операция «инверсия моей», применённая поверх текущего
  состояния, (в) проверка, что цель ещё существует (иначе — no-op с уведомлением).

## 6. Сериализация и round-trip

**`.pcraft`** (`crates/format/src/lib.rs:1-20`): zip (STORE) или каталог;
`manifest.json` (версионированная serde-модель `DocM`, отдельная от `Document`) +
`tiles/<blake3>.zst` (256², little-endian samples) + `blobs/<blake3>.zst` (ICC, EXIF,
PSD-блоки, смарт-объекты) + `thumb.png` / `composite/preview.png` / `composite/full.png`.
Сохранение инкрементальное: `PcraftWriter` помнит уже сжатые тайлы, пишет только новые и
делает GC (`crates/format/src/store.rs`). Лимиты: сторона 300 000 px, площадь
`MAX_CANVAS_PIXELS` (`crates/format/src/lib.rs:152-158`). В манифесте нет времени
создания — при одном билде и одном содержимом байты предсказуемы (по коду; не проверял
сборкой).

**PSD/PSB** — `craft-psd` из craft-libs (standalone, без зависимостей на workspace,
`docs/architecture.md:361-407`), маппинг `doc ⇄ psd` — `crates/io/src/psd_{import,export}.rs`
(52 330 строк `io` всего). Round-trip по `docs/file-format-parity.md:26`: «round trip ≥
169/170, **308/309**, 258/258» (io / psd-tools / Photoshop наборы); oracle floors
190/258 Photoshop-authored, 249/309 psd-tools. В TASK.md написано «307 из 309» — в
документации upstream сейчас 308/309.

**Что не моделируется и как сохраняется:**

| Не моделируется | Как выживает | Риск при конкурентной правке |
|---|---|---|
| Неизвестные PSD additional-layer-info блоки | `Layer.psd_blocks: Vec<([u8;4], Arc<Vec<u8>>)>` (`doc/lib.rs:713-720`), пишутся обратно verbatim | блок перегенерируется, если перестал декодироваться в текущие параметры (`io/src/psd_export.rs:495-500`); после мержа двух правок «extras» могут потеряться |
| Неизвестные image resources / global blocks | `Metadata.psd_resources`, `psd_global_blocks` | не трогаются, пока не трогают документ |
| Тип текста | `TextLayer.psd_raw` + `TySh`, при отсутствии — текст пишется **пикселями** с предупреждением (`psd_export.rs:415-430`) | правка текста на клиенте без PSD-данных → потеря редактируемости |
| Area type | сохраняется как paragraph text по bounds пути (`psd_export.rs:417-419`) | то же |
| Плагинные/неизвестные smart-фильтры | `SmartFilter{command, params}` остаётся verbatim, помечается «not editable» | если плагин не установлен у второго клиента — фильтр пропускается, кэш остаётся (`docs/plugins.md:29-31`) |
| Linked smart objects | `SmartSource::Linked{path}` | путь локален для машины; commit на сервере не должен читать чужие файлы (`automation/src/headless.rs:411-415`) |
| Layer comps, video timeline, variables, slices | есть в модели, частично в PSD | правки в них — те же команды, но кадры видео и XMP большие |
| RAW | открывается как развитый документ (`craft-develop`/`crates/raw`) | исходник не хранится как «RAW + develop-стек» в общем случае |

**Что будет после «прочитать → записать без правок»**: для `.pcraft` — побайтово
эквивалентный набор тайлов (content-addressed) при том же билде; для PSD — «байт-стабильный
round-trip неизменённых файлов» заявлен и покрыт тестами (`docs/architecture.md:399-401`),
но при любом редактировании слоя соответствующий блок регенерируется.

## 7. Структурные правки и конкурентность

| Операция | Что в коде | Что ломается при двух редакторах | Цена пересчёта |
|---|---|---|---|
| Вставка слоя | `insert_above(Option<LayerId>)`, id выдаёт `LayerId::fresh()` | два `fresh()` дают одинаковые id (счётчик с 1 на процесс); порядок при вставке «в одно место» решается только порядком сервера | O(layers) на клон + перекомпоновка bounds |
| Удаление слоя | удаление из `Vec<Layer>`; id не переиспользуется | удаление слоя, который правит другой: команда второго упадёт (`layer {id} not found`) или потеряет правки | O(1) + damage по bounds |
| Перестановка | `layer.moveTo{target, position}`, `layer.arrange.*` | два `moveTo` одного слоя — нужен детерминированный порядок; id-якорь снимает индексный сдвиг, но не гонку двух якорей | то же |
| Пиксели слоя | `Surface` COW-тайлы, `Arc::make_mut` | два актора в одном тайле: без версии тайла второй `make_mut` перетирает первый; нужен merge на уровне пикселя/тайла | только изменённые тайлы: GPU перегружает страницу, если сменился `Arc` (`crates/gpu/src/lib.rs:5-12`) |
| Маска | отдельная `Surface` в `LayerMask` | то же, но независимо от пикселей слоя — цели уже разделены | как пиксели |
| Стиль слоя | `Effects` — набор полей `Layer` | две правки разных эффектов одного слоя — это одна «структура», LWW потеряет одну | пересчёт карт эффектов по damage + reach (`crates/gpu/src/lib.rs:13-19`) |
| Смарт-объект | `contents_id` (общий для копий), `source`, `transform`, `smart_filters` | замена contents (`layer.smartObjects.replaceContents`) у одного и правка transform у другого: кэш и `psd_raw` устаревают, нужен единый писатель | пере-рендер смарт-объекта: до 41 байт/пиксель пика (`memory_gate.rs:25`) |
| Текст | `TextLayer` + `cache` | два набора шрифтов → разные `cache` при одинаковых параметрах | шейпинг + растеризация слоя |
| Selection / quick mask | `Document.selection`, `quick_mask` — глобальные | выделение одного видно другому и попадает в историю | damage по bbox |
| Слои-компы / video | `layer_comps`, `timeline` | те же команды, крупные данные кадров | полная перекомпоновка |

Практический вывод: **структура адресуема id-якорями**, поэтому конфликт «вставили в одно
место» решается порядком сервера, а не OT по индексам. Настоящая проблема — пиксели
одного тайла и то, что `Layer`/`Effects`/`TextLayer` — «широкие» значения: правка одного
поля перезаписывает весь объект.

## 8. Производительность и стоимость пересчёта

Измерено в upstream (их бенчмарки/бюджеты), мной **не** перепроверено:

- `perf/budgets.toml:112-300`: интерактивные правки слоёв на 20 МП документе —
  бюджеты 10–25 ms (move/opacity/fill/blend/visibility/undo), но у P1–P12 стоит
  `enforce = false` («over budget in the 2026-10-05 baseline»), т.е. фактические числа
  хуже бюджета. Открытие 20 МП layered PSD: 3 000 ms (P19), 1 000 ms до первых пикселей
  (P18, over budget).
- Цели v1 (`docs/architecture.md:498-501`): штрих до пикселя за кадр 120 Гц; pan/zoom на
  100 МП/30 слоёв на частоте дисплея; открыть 100 МП JPEG < 1.5 s; 20k×20k 16-bit на 16 GB.
- Память: RAM-бюджет тайлов 2 GiB native / 256 MiB wasm (`crates/raster/src/scratch.rs:23`);
  Memory Usage по умолчанию 8 192 MB, на web потолок 1 536 MB
  (`crates/engine-core/src/prefs.rs:728-737`); GPU: резиденты 4 GiB, эффекты 1.5 GiB,
  паттерны 256 MiB (`crates/gpu/src/consts.rs:23-35`); при превышении — «full refresh on the
  CPU» (`crates/gpu/src/paging.rs:63`).
- 100 МП RGBA8 = 400 MB = 1 600 тайлов на слой (`docs/development.md:697` приводит ту же
  арифметику для 100 МП: «about 400 MB»).

Что пересчитывать при чужой операции:

- Пиксельная дельта одного тайла: CPU — перекомпоновка damage-ректа
  (`crates/engine-core/src/damage.rs:448 step_damage`, bbox изменённых тайлов);
  GPU — пере-аплоад только изменившихся тайлов (сравнение `Arc` по адресу) и повтор
  пассов выше checkpoint'а (`crates/gpu/src/lib.rs:5-25`). Дельта тайла уже имеет готовый
  примитив: `each_changed_tile` сравнивает две `Surface` merge-walk'ом по
  `BTreeMap` и отдаёт изменённые координаты (`damage.rs:12-40`).
- Параметр слоя (opacity/blend/visible): зависит от того, что выше/ниже — при `clipped`
  и pass-through группах может потребоваться весь стек; цена — как P2–P5 выше.
- Структурная правка: полный ре-план/перекомпоновка затронутого ректа.

## 9. Существующие хуки для collab

Есть:

- `Session::execute` + `journal` (id+params) — фактически журнал операций;
- `Session::edit`/`edit_with` — единственный путь мутации документа, с меткой шага и
  коалесингом;
- `Headless` (`crates/automation/src/headless.rs:40-53`): сессия + `PcraftWriter` +
  `save(index, path, format, opts)` — headless-сервер уже умеет открыть, выполнить
  команду, сохранить `.pcraft`/PSD/PSB/ORA (`:369-450`);
- `photocraft-cli serve` — JSON-lines сервер с тем же конвертом, что control-канал
  (`crates/automation/src/rpc.rs:1-20`), auth/лимиты в `crates/automation/src/security.rs`;
- revision-счётчики вместо шины событий: `DocState.revision`/`last_damage`
  (`crates/engine-core/src/doc_state.rs:17-27`), `documents_generation`,
  `active_layer_changes`, `Tracked` из craft-libs. **Шины событий (`EventReceiver`,
  `DocChanged`) в коде нет** — она есть только в плане (`docs/architecture.md:196-204`).
- `SmartFilter{command, params}` — уже сериализуемая неразрушающая операция;
- плагины-wasm (`plugin.run`) — фильтры, не транспорт.

Чего нет и придётся добавить:

- сетевого id-пространства (актор-диапазон или выдача id сервером);
- версии документа/операции (Lamport или серверный счётчик) и идемпотентности;
- per-user undo (инверсии/журнал на актора);
- presence (курсор/вьюпорт/активный слой): в документе этого нет, живёт в `ToolState` и
  UI; в браузере нет ни WebSocket, ни TCP-сервера (`apps/photocraft-web/src/main.rs:1-16`:
  «no TCP control server (browsers can't listen on sockets)»);
- сериализации тайл-дельты (сейчас `.pcraft` умеет только полный набор тайлов, а
  `each_changed_tile` — приватный и отдаёт bbox, не список);
- правила для производных кэшей (text/shape/smart/fill cache) — какие поля не мержатся.

## 10. WASM / web

- `apps/photocraft-web` — eframe web runner, wgpu (WebGPU, fallback WebGL2), trunk,
  профиль `wasm-release` (`apps/photocraft-web/Cargo.toml:1-45`, `Trunk.toml`).
- Модуль ограничен по размеру: `packaging/web/package.sh` валит сборку выше 24 MiB;
  после `wasm-opt -Oz` ~71.6 MB raw против 69.7 MB fat-LTO (`Cargo.toml`, комментарий к
  `[profile.wasm-release]`) — т.е. бюджет байтов жёсткий, и любой сетевой стек надо
  вписывать в него.
- Потоки: `rayon` только под `cfg(not(target_arch = "wasm32"))`
  (`crates/algo/Cargo.toml:25-26`, `crates/compose/Cargo.toml:18`); `SharedArrayBuffer`,
  `wasm-bindgen-rayon` в репозитории не используются — **wasm однопоточный**.
- Память: линейная память wasm ≤ 4 GiB, Memory Usage на web капается 1 536 MB
  (`prefs.rs:733`), RAM-тайлы 256 MiB (`scratch.rs:23`), любая большая операция сначала
  спрашивает `headroom::ensure` (пробный `try_reserve_exact`), потому что неудачный
  infallible-alloc в wasm убивает модуль (`crates/raster/src/headroom.rs:1-20`).
  Для документа 100+ МП (400 MB пикселей на слой) браузер — заведомо не место: даже один
  слой с запасом на композит упирается в 1.5 GiB.
- WebSocket: в зависимостях нет (`web-sys` фичи в `Cargo.toml` перечислены без
  `WebSocket`), транспорта нет; control-сервер на loopback в браузере невозможен.
- GPU в браузере есть (WebGPU/WebGL2), но CPU-фолбэк обязателен: `?cpu`, `?webgl`,
  `?free-mb=N` — диагностические флаги (`apps/photocraft-web/src/main.rs:14-20`).

Вывод для лимитов: 100+ МП — это desktop/сервер; браузер — сессия с лимитом документа
(например, ≤ 24–36 МП) и явным отказом выше; память клиента считается как
«документ × коэффициент истории», иначе вкладка падает без сообщения.

## 11. Три варианта протокола (кратко)

| Вариант | Механизм | Плюсы здесь | Минусы здесь | Риск |
|---|---|---|---|---|
| A | Авторитетный сервер + журнал команд: команда идёт на сервер, он исполняет её в `Session`, рассылает подтверждённую команду + тайл-дельты, версия серверная | совпадает с единственной точкой мутации (`Session::execute`), `Headless` уже есть, commit одним писателем, `.pcraft` детерминирован | сервер должен держать документ и исполнять и кисти, и фильтры; per-user undo = инверсии на актора | стоимость сервера на 100 МП (память+CPU композита) |
| B | Операционные дельты с OT: клиент шлёт примитивы (`SetLayerProps`, `PaintTiles`, `MoveLayer`), сервер трансформирует против конкурентных | минимальный трафик, сервер не исполняет алгоритмы | нужен второй слой мутаций в обход `Session::edit`; инверсия каждой операции; OT-правила для 33 полей `Document` и 23 `Layer` | расхождение с командами (два источника истины) |
| C | CRDT на подмножестве: дерево слоёв как sequence-CRDT, свойства LWW, тайлы LWW-регистры | сходимость без серверной очереди, offline | `Document` — `Vec<Layer>` без позиционных id; undo в CRDT отдельная семантика; пиксели в одном тайле LWW = потери (TASK §5 запрещает) | переписывание модели и производных кэшей |

Подробно, включая пять обязательных проверок (тайлы, per-user undo, структура, PSD,
100+ МП) — `options.md`.

## 12. Риски и открытые вопросы

1. **Пиксели одного тайла.** В коде нет версии/хэша тайла, только `Arc`-идентичность и
   `generation` на всю поверхность; merge «по реально изменённым пикселям» (TASK §5)
   требует либо пиксельного штампа внутри дельты, либо авторитетного порядка и
   детерминированного «последний по серверу» с сохранением чужих пикселей.
2. **Per-user undo несовместим с `ops` как есть.** Снапшот на транзакцию — глобальный;
   инверсий нет; `undo` вернёт чужое. Нужен журнал операций с инверсиями и проверкой
   существования цели.
3. **Широкие значения.** `Layer`, `Effects`, `TextLayer`, `SmartObject` — объекты, а не
   поля; две правки разных полей одного слоя конфликтуют на уровне объекта.
4. **Производные кэши в документе.** `text/shape/smart/fill cache` — растр, зависящий от
   шрифтов/GPU; нельзя мержить, нужно правило «кэш пересчитывает один писатель».
5. **PSD round-trip после мержа.** 308/309 достигнуто для одиночного редактирования;
   после слияния двух правок одного adjustment/type-слоя блок регенерируется и «extras»
   (`CgEd` и пр.) могут потеряться (`io/src/psd_export.rs:495-520`). Нужен тест
   «два клиента правят один слой → commit → round-trip не хуже 308/309».
6. **Стоимость снапшота на 100 МП** — арифметика (48 000 Arc-указателей на снапшот), но
   реальное время `Document::clone` и цена 50 состояний истории не измерены; мерить на
   хосте `agents` до выбора лимитов.
7. **Транспорт в браузере** отсутствует полностью (нет WebSocket, нет потоков);
   backpressure presence придётся строить с нуля.
8. **Не проверено:** фактическое время `compose`/GPU-композита на 100 МП, размер
   типичного `.pcraft`, поведение `each_changed_tile` на разреженных документах,
   детерминизм `paint.stroke` между платформами (f32-порядок операций в кисти).
