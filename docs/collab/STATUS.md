# PhotoCraft — состояние проекта

Раздел 5 — хронология (новое сверху). Один датированный раздел на проход, ≤30 строк.

## 5.3 — 2026-11-25 — Phase 1: ADR-0001…0010 сверены, адаптер начат (тайлы + per-user undo)

- Phase 0 закрыта: `collab/study` ребейзнут на `origin/main` 13520900d и
  запушен в `copoka/photocraft`; study/options опубликованы `artifact`.
- Соответствие принятому B зафиксировано в `docs/collab/adr-conformance.md`
  (10 ADR → код PhotoCraft): все ложатся без второго движка; единственное
  дополнение к конверту — `TileDelta::Patch` (разреженные пиксели), иначе
  TASK §5 «в одном тайле без потерь» не выполняется.
- Начат адаптер `crates/collab-adapter` (L2, `collab/adapter` 3937cb9db,
  форк): `diff`/`apply` тайловых дельт по правилу `each_changed_tile`
  (сравнение COW-указателей, неизменённые тайлы не читаются), маска — отдельная
  часть; `UndoLog` per-actor проверяет только изменённые пиксели, поэтому undo
  A не откатывает B, а пересечение по пикселю — `UndoRefusal::Conflict`.
- Гейты на agents: `cargo test -p photocraft-collab-adapter` 8/8,
  `clippy --all-targets -- -D warnings` чисто, `cargo xtask layers` 31/31 OK,
  `cargo fmt --check` чисто. `docs/collab/adapters.md` — шесть поверхностей и
  следующие инкременты.
- PR в `storytold/photocraft`: #16984 (изучение, соответствие ADR, план,
  STATUS) и #16985 (крейт адаптера) — оба MERGEABLE, CI-раны форка не
  запущены, на #16984 висит внешний CodeRabbit.
- Не измерено: размер `Patch` на реальном штрихе, `Document::clone`/compose на
  100 МП, детерминизм `paint.stroke`; PSD-writer на сервере не подключён.

## 5.2 — 2026-11-25 — Phase 0: study/options готовы, рекомендуем B

- Написаны `docs/collab/study.md` (10 вопросов + «что не моделируется / round-trip»)
  и `docs/collab/options.md` (A: клиенты+OT, B: авторитетный сервер, C: CRDT).
- Измерено по коду: 791 111 строк Rust в 2 251 src-файле (907 052 с тестами);
  1 018 команд в `command_ids.txt`; `Document` — 33 поля, `Layer` — 23, `Session` — 53;
  `crates/doc` — 87 структур / 46 enum.
- Ключевые факты для протокола: `Session::execute` — единственная точка мутации;
  история `crates/ops` — полные снапшоты `Arc<Document>` (50 состояний, лимит =
  Memory Usage), per-user undo на них не строится; `LayerId` — процессный счётчик с 1,
  нужен актор-диапазон; вставка/перемещение уже id-якорные (`insert_above`,
  `layer.moveTo{target}`); `each_changed_tile` даёт изменённые тайлы из before/after;
  `.pcraft` content-addressed, commit одним писателем через `Headless::save`;
  PSD round-trip 308/309 (`docs/file-format-parity.md:26`).
- Риски: пиксели в одном тайле без версии тайла; производные кэши (`text/shape/smart/
  fill cache`) внутри документа; широкие значения `Layer`/`Effects`; PSD-блоки
  регенерируются после мержа. Не измерено: `Document::clone`, compose/GPU на 100 МП,
  размер типичного `.pcraft`, детерминизм `paint.stroke` между платформами.
- Дальше: публикация study/options как `artifact`, proposal по единице операции,
  ревью proposals соседей и голоса по 10 измерениям.

## 5.1 — 2026-11-25 — проект заведён, Phase 0 открыта

- Каталог `photocraft/` создан в `cabinet-office` по образцу остальных проектов:
  `TASK.md`, `INTEGRATION.md`, `KICKOFF.md`, `AGENTS.md`, `opencode.json`,
  `.opencode/agents/photocraft-envoy.md`.
- `upstream/` — клон `storytold/photocraft` (`main`, `830316b77`, 0.23.1).
- Снято с upstream (проверять в study, а не принимать на веру):
  `crates/doc` — `Document`/`Layer`/`LayerId(u64)`, маски, корректирующие слои,
  стили, текст, векторы, смарт-объекты; `crates/ops` — история как **полные
  снапшоты документа** на транзакцию (COW-тайлы в `Arc`) с лимитом состояний и
  байтовым бюджетом; `crates/format` — `.pcraft`; PSD/PSB — `craft-psd` из
  craft-libs; `apps/photocraft-web` — trunk/wasm32.
- Совместного редактирования в upstream нет; host-bridge (`host=parent`) не
  реализован.
- Дальше: `upstream/docs/collab/study.md` + `options.md`, `artifact` в
  `study/photocraft`, proposal по единице операции, ревью и голоса.

## Next session

1. Свойства слоя дельтами (`visible`, `opacity`, `blend`, `name`, `locks`,
   порядок) и их инверсии в `UndoLog`; structural-конверт: создание/удаление
   слоя, маска, смарт-объект; выдача `LayerId` сервером (ADR-0002).
2. Seam-трейт Session + mock-relay (как `wordcraft-collab-adapter`), затем
   снапшот `.pcraft` из авторитетного документа и commit-callback.
3. Сверить с контрактами `craft-crdt` (#345) и `craft-libs#160`, когда они
   появляются в пине; заменить локальный seam на общий, не меняя документную
   половину адаптера.
4. Измерить на agents: размер `Patch` для реального штриха, `Document::clone`,
   compose на 100 МП, детерминизм `paint.stroke` между платформами.
