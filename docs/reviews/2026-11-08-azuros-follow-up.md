# Afiocira — targeted follow-up Азу́роса Клаудапи

Область: восемь замечаний [первого ревью](<../../docs/reviews/2026-11-08-azuros-architecture.md>), новые selection/private-protocol/Settings/cache contracts и UI Foundation. Это проверка ремонта, не повторный аудит всего проекта.

## Найденные при перепроверке регрессии

### [P2] Отмена не останавливала отложенный Rename — исправлена и независимо перепроверена

В первом представленном follow-up binary `e0b9531d…` последовательность «own Yazi paused → `rename renamed.png` ожидает snapshot → `cancel_rename` success → resume Yazi» действительно переименовала `a.png`, хотя отмена уже завершилась. Отмена обнуляла draft, но continuation оставался в navigator state. [Доказательство до исправления](<../../target/azuros-review/native-15b244c6/results-follow-up.json>).

Теперь [navigator.rs:45](<../../crates/photo-gui/src/navigator.rs#L45>) сопоставляет конкретную cancel-команду с ожидающей continuation, [navigator.rs:78](<../../crates/photo-gui/src/navigator.rs#L78>) удаляет её, отменяет snapshot ID и отвечает исходному IPC caller явной ошибкой. Включение происходит до нового dispatch ([dispatch.rs:5](<../../crates/photo-gui/src/dispatch.rs#L5>)).

На binary `ea0a4a788694…` и повторно на `cbe4e7989724…` независимо проверены Rename/CancelRename, RunAction Delete/CancelDelete и ShowCommands/CloseCommands. Ответ `Pending navigator command was canceled` получен ещё до возобновления Yazi; после поздней доставки не появились rename/delete drafts, palette или operation. Оба synthetic файла сохранились. Несоответствующий `CancelDelete` не отменил ожидающий Rename. [Последний результат после исправления](<../../target/azuros-review/native-54edd698/results-follow-up-final.json>).

### [P2] Корректный CAS merge создавал конфликтующий полный профиль — исправлена и независимо перепроверена

Два открытых экземпляра могли локально валидно подготовить разные Quicklink IDs `a`/`b` с одинаковым `g → 1`. После последовательных Apply оба сохранялись enabled, effective keymap оставлял A, а `TestQuicklink(b)` открывал папку A. Сохранность отдельных полей ещё не доказывала валидность объединённого профиля. [Доказательство до исправления](<../../target/azuros-review/native-3578aaca/results-follow-up.json>).

Теперь полный результат проходит semantic validation перед возвратом prepared candidate ([yazi_profile.rs:211](<../../crates/photo-gui/src/yazi_profile.rs#L211>)), под lock перед сохранением ([yazi_profile.rs:233](<../../crates/photo-gui/src/yazi_profile.rs#L233>)) и перед генерацией TOML ([yazi_profile.rs:569](<../../crates/photo-gui/src/yazi_profile.rs#L569>)). Проверяются активные key/prefix overlaps между Quicklinks и bindings; ID namespaces этих двух списков по принятому решению независимы, дубли внутри каждого списка запрещены.

Независимый native повтор: B Apply отклонён с `overlap`; persisted A, staged draft B и старый live B не изменились. После смены B на `g → 2` объединение обеих записей успешно, а `TestQuicklink(b)` реально открыл destination B. Сценарий повторно прошёл на `cbe4e7989724…`. [Последний результат после исправления](<../../target/azuros-review/native-54edd698/results-follow-up-final.json>).

## Состояние исходных восьми замечаний

| № / прежняя severity | Состояние | Основание |
| --- | --- | --- |
| 1 / P1: неснятые descendant marks | **Исправлено** | Lua считает delta по actual selection внутри loaded+historical owned scope и использует explicit URL on/off. Native contract test проверил A+B+C → A+C → C, сохраняя внешний C. |
| 2 / P2: replace вытесняется preserve | **Исправлено** | Immutable per-ID requests, последовательный consumer, ACK после проверки actual state; dirty сбрасывается только по ACK совпадающих marks той же collection. Native test включал paused consumer, replace→preserve и replace→replace. |
| 3 / P2: старый snapshot после timeout | **Исправлено** | Новый logical read получает новый ID/path; cancel tombstone, поздний response собирается без импорта. Handoff стал pending command с deadline/revision guard и held reply; отдельный Status не блокируется ожиданием. Native ID-retirement test и cancellation probes прошли. |
| 4 / P2: потеря чужих Settings | **Исправлено в проверенных сценариях** | Fresh read под OS lock, field/row CAS и full-profile validation. Старый repro shortcuts теперь сохраняет J и L; multi-instance conflicting/disjoint Quicklinks проверены независимо. |
| 5 / P2: старые pixels после atomic replace | **Исправлено** | Known-path membership events инвалидируют affected preview с generation retirement; остальные textures сохраняются. В моём повторе texture `14 → 16` без F5, затем manual refresh `17`; bytes действительно заменены. |
| 6 / P2: Settings runtime restoration через DDS | **Исправлено по коду и native preference test** | `emit` идёт в тот же private stream с positional/named args; фактические sort/hidden/linemode подтверждаются snapshot. Candidate transport errors наблюдаются. Отдельные fail/hang `ya.exe` native проверки выполняются основным агентом и должны быть учтены как его результаты, а не мои. |
| 7 / P2: Delete отвергает descendants | **Исправлено** | Captured SelectionScope проверяет collection ID и точные members. Native group confirm для двух альбомов принят; sources заранее сделаны stale, и worker корректно отверг их до Shell, не трогая Recycle Bin. |
| 8 / P2: неограниченный IPC | **Исправлено в заявленных bounds** | 8 clients, queue 8, request 16 MiB, read/write 3 s, reply 15 s, bounded GUI drain между commands. Четыре IPC tests прошли, включая admission/deadline/release; nonblocking close не оставляет blocking flush pool. Отдельный filesystem command всё ещё может быть медленным — это прежнее архитектурное ограничение, не гарантия frame ≤4 ms. |

Доказательства дополнительных native повторов: [shortcuts](<../../target/azuros-review/native-21186ded/results-follow-up.json>), [same-path replacement](<../../target/azuros-review/native-b494d0c1/results.json>), [descendant confirmation без Shell](<../../target/azuros-review/native-3cfab4a7/results-follow-up-scope.json>).

## Проверка новых контрактов

**Private protocol.** Публикация отделена от ACK; сам ACK проверяет observed cursor/cwd/selection/preferences. Capacity 64 и размер 4 MiB ограничивают accumulation; неопределённая мутация poison-ит stream до restart. Отмена чтения не освобождает sequence slot раньше consumer retirement. Просроченная live mutation за cancelled front теперь также обнаруживается. Это консервативное и понятное поведение. Snapshot ownership между Settings и navigator разделён `navigation.busy` guard; импорт результата ограничен continuation revision. Найденная мной отсутствовавшая отмена continuation устранена отдельно, а не маскируется общим timeout.

**SavedPreview/cache.** Receipt строится из verified encoded pixels; JPEG preview использует именно decoded quality-95 output, не исходные pixels до lossy encode. `PublishedFile` сохраняет SourceVersion проверенной staging file до rename. UI upload ограничен актуальными operation ID, collection, path и `is_current()` ([operations.rs:578](<../../crates/photo-gui/src/operations.rs#L578>)); при внешней замене используется обычный decode. Fallback texture отделена от ready cache и исчезает при свежем preview/error, а не считается подтверждением новой версии. Unit tests проверили pixel identity PNG/JPEG и отказ receipt после replacement с идентичными байтами, но другой file identity. Эти проверки не равны гарантии отсутствия любой внешней записи между metadata check и paint.

**UI Foundation.** Общие tokens и SettingsRow/Section/Control/command_row реально используются через существующие wrappers. Native egui Response, focus IDs и enabled states сохранены; component не вводит второй action dispatcher. Два geometry/input tests прошли: width/DPI matrix, длинные русские labels, реальные widget rects, 32-point minimum, переносы, click/focus. Settings capture честно возвращает InvalidState на неподдерживаемом immediate viewport; выдавать root screenshot за Settings screenshot больше не предполагается. Внешний вид шрифтов, контраст и физическое OS input routing этими тестами не сертифицированы.

## Лично выполненные проверки

**21 различный focused test**: UI Foundation 2; navigator matcher/album return 2; private protocol 3; IPC 4; saved PNG/JPEG preview 1; publication receipt 1; core SelectionScope 2; Settings profile merge/validation 4; bounded reader 1; real native private-protocol contract 1. Native contract повторён после изменения plugin entry и дополнен проверкой task accounting. Его запуск имел отдельные `AFIO_API_NAME`, `AFIO_SETTINGS_HOME` и `YAZI_CONFIG_HOME` и приостанавливал только свой дочерний Yazi.

Native проверки моих scripts использовали уникальные synthetic configs/media/API names и только собственные process trees. Последний полный cancel/CAS повтор выполнен на GUI SHA-256 `cbe4e7989724645741b7e1ca42ce7994dbef13d5457409b5b0780ddc2f17d5fc`. [Harness](<../../target/azuros-review/check_follow_up_final_connect.py>) повторяет только открытие pipe при transient errno 2/22 до отправки любых байтов; опубликованные мутации не пересылаются. В предыдущей версии harness такой connect failure мог завершить original future до enqueue, пока другой поток напрасно ждал `navigator.pending`; этот Timeout не предъявляется продукту. Также исправлялся вывод Unicode в console CP1251 после успешной записи JSON. Последний запуск завершился exit 0.

Первый ошибочный test filter `yazi_profile::tests` запустил 0 tests; он не учитывается. После уточнения имени `yazi_profile::persistence_tests` лично выполнены все 4 нужных теста. Полные workspace/Clippy/release/native suites и latency benchmark основного агента в мои результаты не включены.

## Финальные узкие изменения и вердикт

[convert_image.rs:27](<../../crates/photo-gui/src/convert_image.rs#L27>) теперь резервирует проверенный размер один раз, а не увеличивает exact capacity на каждые 64 KiB. Лимит 64 MiB проверяется до allocation; рост отвергается до append, усечение — на EOF. Source identity/version, сравнение bytes, no-clobber staging и rollback сохранены. Новый large-payload/growth/shrink test прошёл. Улучшение latency здесь имеет конкретную причину — устранены повторные копии растущих буферов; собственных численных сравнений производительности я не выполнял.

[yazi.rs:93](<../../crates/photo-gui/src/yazi.rs#L93>) задаёт поддерживаемую `@sync entry`, которая запускает async callback без помещения самого bridge в очередь пользовательских jobs. После изменения лично повторён enhanced native contract: 12 последовательных polling snapshots имеют `tasks=0`, а настоящий synthetic user plugin попадает в task summary. Guard не исправлялся игнорированием реальных tasks. Следующая небольшая доработка Settings Snapshot phase просмотрена статически: она свежими snapshots ждёт фактический `tasks=0` в прежнем общем deadline 5 s; dialogs отказывают немедленно, long-running tasks дают явную ошибку без подмены pane. Repaint 10→25 ms не меняет request/revision/deadline semantics. Native suites этой последней waiting-phase доработки выполняет основной агент.

**Вердикт Клаудапи: подтверждённых блокирующих замечаний к проверенному ремонту не осталось. Восемь исходных замечаний закрыты в описанном объёме; обе новые найденные мной P2 также исправлены и независимо перепроверены.** Изменения можно включать в рабочую сборку после завершения её текущих native/build checks. Это технический вывод ревью, а не новая процедура согласования с пользователем.

Остались **неблокирующие ограничения проверки**: физические клавиши и OS focus; визуальная приёмка Settings при неподдерживаемом native screenshot; настоящий clipboard; crash/power-loss; latency на иных codec/cloud/размерах. Отдельный slow filesystem command способен удержать UI, несмотря на лимит drain между командами; этот старый системный долг текущим ремонтом не заявлен исправленным. Root-frame cache screenshots и численные latency результаты основного агента следует сообщать с их реальным fixture и binary, без универсального обещания ускорения.

Контрольные hashes проверенных source/config: [follow-up-source-sha256.json](<../../target/azuros-review/follow-up-source-sha256.json>). Native свидетельства привязаны к указанному выше CBE binary; последующий release fingerprint и результат общей финальной проверки фиксирует основной агент.

Исходники продукта в ходе ревью не редактировались. Исходное whole-project ревью остаётся справедливым в части долгосрочных границ: session/application service, shutdown/job contracts, full-resolution/color pipeline и воспроизводимый runtime bundle. Их не следует путать с повторным запретом нынешнего ремонта или основанием для немедленного rewrite.
