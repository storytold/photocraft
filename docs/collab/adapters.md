# PhotoCraft adapter to RFC-0020 — plan and first increment

- Дата: 2026-11-25
- Ветка: `collab/adapter` (от `origin/main` 13520900d)
- Крейт: `crates/collab-adapter` (`photocraft-collab-adapter`), Class L2 в
  `xtask/src/layers.rs` (doc + raster + geom + color, без движка и UI)
- Соответствие ADR: `adr-conformance.md`

## Шесть поверхностей адаптера

1. **Session.** Единственная точка мутации — `Session::execute`
   (`crates/engine-core/src/session/execute.rs:14`). Адаптер не переписывает
   движок: он нормализует конверт (явный слой, `target`, точки, `seed`,
   кисть), классифицирует операцию (`command`/`primitive`/`structural`) и
   считает результат из пары `before`/`after`, которую движок уже держит
   (`edit_with`, `execute.rs:239`). Порядок, дедуп `opId`, lease и роль — у
   сервера.
2. **Deltas.** `diff(&Document, &Document) -> ChangeSet`: пиксели и маски —
   `TileDelta::Patch` (разреженные пиксели + их новые байты), целый тайл —
   `TileDelta::Replace` (новый тайл, смена формата). `each_changed_tile`
   (`crates/engine-core/src/damage.rs:11`) остаётся в движке внутренним; его
   правило (сравнение указателей COW-тайлов, без чтения пикселей) адаптер
   повторяет для `Surface`, потому что ему нужны не только координаты, но и
   сами изменённые пиксели. Производные кэши (`text/shape/smart/fill`) в дельту
   не входят.
3. **Undo.** `UndoLog` на актора: шаг хранит before/after-образы затронутых
   тайлов и отпечаток тайла после шага. Undo проверяет отпечаток, строит
   обратную дельту и применяет её как новую операцию; чужой тайл после шага →
   `undo-conflict`. `crates/ops` не трогаем.
4. **Presence.** Курсор, выделение, активный слой, вьюпорт — отдельный кадр,
   не журналируется; в первый инкремент не входит (нет транспорта).
5. **Commit.** `Headless::save` на сервере; адаптер отдаёт авторитетный
   `Document`, не байты файла. Первый инкремент не коммитит.
6. **Host.** `photocraft:session`/`presence`/`commit` в iframe кабинета —
   после клиентского транспорта и общей оболочки.

## Первый инкремент (этот проход)

`crates/collab-adapter`:

- `TileAddr { layer: LayerId, part: Part, coord: TileCoord }` — адрес тайла,
  маска отдельно от пикселей.
- `TileDelta::{Replace, Patch}`; `Patch` несёт только изменённые пиксели
  (`(index, bytes)`), поэтому две кисти в одном тайле не теряют пиксели друг
  друга при наложении на серверный тайл.
- `diff`, `apply`, `fingerprint`.
- `UndoLog` (per-actor, undo/redo, лимит шагов, `undo-conflict` по отпечатку).
- Тесты: `tests/it/main.rs` — один бинарник, пять сценариев (два актора в разных
  тайлах, два актора в одном тайле, маска отдельно, redo, конфликт undo).

## Что осталось до Phase 1 (по возрастанию цены)

| шаг | где |
|---|---|
| Свойства слоя как дельта (`visible`, `opacity`, `blend`, `name`, `locks`, порядок) и обратные операции | адаптер |
| `Structural`-конверт: создание/удаление слоя, маска, смарт-объект; выдача `LayerId` сервером | адаптер + сервер |
| Сжатие `Patch` (zstd) и лимит кадра | адаптер (wire — общий слой) |
| Seam-трейт Session и mock-relay, как в `wordcraft-collab-adapter` | адаптер |
| Снапшот `.pcraft` из авторитетного документа, commit-callback | сервер + `Headless` |
| Транспорт WebSocket (native + wasm), presence, панель участников | клиент |
| Conformance «два браузера», 100+ МП, PSD round-trip после мержа | тесты |

## Гейты

- `cargo test -p photocraft-collab-adapter`
- `cargo clippy -p photocraft-collab-adapter --all-targets -- -D warnings`
- `cargo xtask layers`
- для затронутого движка (если понадобится) — `cargo test -p photocraft-engine`

Все запускаются на хосте `agents` через `../scripts/remote-build.sh`.
