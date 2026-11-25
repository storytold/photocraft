# PhotoCraft — состояние проекта

Раздел 5 — хронология (новое сверху). Один датированный раздел на проход, ≤30 строк.

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

1. Прочитать `KICKOFF.md` (TASK-000) и шаблоны `.craft/templates/`.
2. Написать study/options, опубликовать `artifact`.
3. Отревьюить proposals соседей по 10 измерениям, проголосовать.
