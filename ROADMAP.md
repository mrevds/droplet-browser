# Droplet Roadmap — Дорожная карта разработки

**Главная цель (North Star):** сверхбыстрый, лёгкий графический браузер на Rust без мусора («очень быстрый браузер без мусора»).  
Каждая фаза спроектирована так, чтобы сохранять минимальный размер бинарника, низкое потребление памяти и субмиллисекундный рендеринг, позволяя следующим AI-моделям автономно брать тикеты из [.scratch/SPEC.md](.scratch/SPEC.md).

---

## Целевые метрики производительности (Performance Targets)

| Метрика | Целевое значение | Примечание |
| --- | --- | --- |
| **Cold Start** | `< 20 ms` | От запуска процесса до первого кадра на `about:droplet` |
| **Idle RAM (RSS)** | `< 15 MB` | На статической странице с одним окном (и десятки MB даже при многих вкладках за счёт Tab Hibernation) |
| **Layout + Paint** | `< 2 ms` | Полный цикл `Style -> Layout -> DisplayList -> Rasterize` для типовой страницы |
| **Idle CPU** | `~ 0%` | Никакого polling-рендеринга, перерисовка только по `dirty`-флагам |
| **Compile Time** | `< 2 s` (incremental) | Ноль внешних крейтов в `Cargo.toml`, строгая модульность без proc-macro |

---

## Правила защиты от раздувания кода (Anti-Bloat Guidelines для AI-агентов)

1. **Не тащить тяжёлые зависимости.** Не подключать `tokio`, `hyper`, `reqwest`, `winit`, `egui`, `chromium`-обёртки или огромные графы крейтов. Во всех крейтах workspace **ноль внешних зависимостей** (только локальные `droplet-*`).
2. **Данные важнее иерархий.** Использовать плоские массивы (`Vec<Node>`, `DisplayList`), `Vec<(String, String)>` для DOM-атрибутов, целочисленные индексы (`NodeId(usize)`) и `Copy`-структуры (`ComputedStyle`) без `HashMap` и без `Rc<RefCell<...>>`.
3. **Разделять этапы конвейера.** Сетевой слой не знает про layout, layout не рисует пиксели, rasterizer не знает про X11/Win32, а весь платформенный FFI изолирован в `crates/droplet-platform/src/` и `#[cfg(target_os = ...)]`.
4. **Бенчмарк перед усложнением.** Любой кэш должен быть жёстко ограничен по памяти (`MAX_ATLAS_ENTRIES = 4096`, `MAX_WARM_TABS = 3`, `DNS_CACHE_MAX_ENTRIES = 256`, `POOL_MAX_TOTAL = 32`).
5. **Обязательное покрытие тестами.** Перед переводом тикета в `done` должны проходить `cargo test`, `cargo clippy --all-targets` и визуальный прогон на `about:test` / `about:droplet`.

---

## Стратегия кроссплатформенного оконного слоя (Cross-Platform Windowing Strategy)

Весь платформенный GUI-слой вынесен в `crates/droplet-platform/src/` (`mod.rs` с `#[cfg(target_os = ...)]`, а `crates/droplet-platform/src/lib.rs` сохранён как compatibility alias):

- **Ubuntu Linux:** X11 (`crates/droplet-platform/src/linux_x11.rs`, **готово** — динамический `dlopen("libX11.so.6")` + `XCreateImage`/`XPutImage`) -> далее нативный клиент Wayland + `MIT-SHM` zero-copy blitting (`DROP-011` / `DROP-009`).
- **Windows:** Win32 API (`crates/droplet-platform/src/windows.rs` + `StretchDIBits`, **готово в базовом виде**) -> далее Windows Schannel TLS (`DROP-011`) + Direct2D/D3D11 ускорение (`DROP-009`).
- **macOS:** архитектурный слот (`crates/droplet-platform/src/macos.rs`), но сборка и живое тестирование Cocoa/Metal требуют macOS/Apple SDK (на Ubuntu сейчас не собирается нативно; доработка вынесена в `DROP-011` при наличии macOS-хоста).

---

## Фазы развития

### Phase 1 (`DROP-001`) — Core Pipeline `[DONE]`
Базовый сквозной конвейер от URL до пикселей на экране без единой внешней зависимости (pure `std`):
- **URL & Net:** парсер URL + разрешение относительных путей (`http://`, `file://`, `about:*`), HTTP/1.1 клиент с таймаутом 5с, `Transfer-Encoding: chunked` декодер, поддержка редиректов (до 5), встроенные страницы `about:droplet`, `about:roadmap`, `about:architecture`, `about:test`, `about:blank`.
- **HTML & Arena DOM:** итеративный стековый HTML-парсер (UTF-8 safe, raw text `<style>`/`<script>`, entities, auto-close `<li>`/`<p>`), кэш-дружелюбный Arena DOM (`Vec<Node>` + `NodeId(usize)`).
- **CSS & Style Tree:** парсер селекторов и правил, расчёт specificity `(id, class, tag)`, каскад и наследование свойств.
- **Layout & Rasterizer:** Box Model + Block/Inline обтекание текста, встроенный 8x16 шрифт (ASCII + полная кириллица), плоский `DisplayList`, программный XRGB-растеризатор, hit-testing ссылок `<a href>`.
- **Shell & Window:** история Back/Forward, динамический X11 backend через `dlopen("libX11.so.6")` и headless CLI (`--dump-dom`, `--dump-layout`, `--screenshot`).

---

### Phase 2 (`DROP-002`) — Full Inline/Inline-Block & Flexbox Layout `[DONE]`
Переход от базового block/inline-потока к современной вёрстке страниц:
- **Реализовано:**
  - `display: inline-block` (смешанное строчно-блочное позиционирование, перенос по строкам).
  - Двухпроходный движок **Flexbox** (`display: flex`, `flex-direction`: `row`/`column`/`row-reverse`/`column-reverse`, `justify-content`, `align-items`, `flex-wrap`, `gap`, `flex-grow`/`flex-shrink`/`flex-basis`).
  - Относительные единицы в `style`/`layout`: `%`, `em`, `rem`, `vh`, `vw`.
  - Интерактивные псевдоклассы `:hover` и `:active` (`StyleContext`, `hit_test_node`, нулевой overhead на страницах без `:hover`/`:active` через `stylesheet_uses_interaction`).
  - `Copy` `ComputedStyle` без `HashMap` и без per-node heap-аллокаций.
- *Спецификация:* [DROP-002-inline-and-flexbox.md](.scratch/issues/DROP-002-inline-and-flexbox.md).

---

### Phase 3 (`DROP-003`) — HTTPS (TLS) & Concurrent Resource Loader `[DONE]`
Выход в современный веб (`https://`) и параллельная загрузка ресурсов без `Cargo.toml`-зависимостей:
- **Реализовано:**
  - `https://` на Linux через runtime `dlopen` системного OpenSSL (`libssl.so.4` / `libssl.so.3` / `libssl.so`), системные CA (`SSL_CTX_set_default_verify_paths`), SNI и проверку hostname (`SSL_set1_host`).
  - HTTP/1.1 **Keep-Alive connection pool** (`POOL_MAX_PER_ORIGIN = 4`, `POOL_MAX_TOTAL = 32`, `POOL_IDLE_TTL = 30s`) с автоматическим переподключением на протухших сокетах.
  - Внутрипроцессный **DNS TTL cache** (60s, max 256 записей) и поддержка `data:` URI (`fetch_uri_bytes`).
  - Многопоточный загрузчик (`fetch_many`, пул до 4 `std::thread` воркеров) для `<link rel="stylesheet">` и подресурсов.
  - Встроенный zero-alloc **tracker/ad domain filter** (`crates/droplet-net/src/adblock.rs` -> `TrackerFilter`) до DNS/TCP.
- **Follow-up (вынесено в `DROP-011`):** нативный Windows Schannel TLS / macOS системный TLS (на не-Linux платформах сейчас возвращается явная ошибка `NetError`).
- *Спецификация:* [DROP-003-https-and-external-css.md](.scratch/issues/DROP-003-https-and-external-css.md).

---

### Phase 4 (`DROP-004`) — Crisp Typography, Image Decoding & Multi-Tab Shell `[DONE]`
Сглаженная типографика на чистом Rust, декодирование изображений и минималистичная мульти-вкладочная оболочка:
- **Реализовано:**
  - Поиск системных TrueType шрифтов на Linux (`/usr/share/fonts/truetype/...`) и Windows (`C:\Windows\Fonts\...`) + pure-Rust `sfnt`/`glyf` парсер и anti-aliased сканлайн-растеризатор (`crates/droplet-font/src/lib.rs` -> `FontEngine`).
  - Ограниченный `GlyphAtlas` (до `MAX_ATLAS_ENTRIES = 4096` записей) + автоматический fallback на вторичные шрифты (кириллица) и встроенный `8x16` bitmap шрифт.
  - Pure-Rust декодер изображений (`crates/droplet-image/src/lib.rs` -> `DecodedImage`): **PNG** (включая Adam7 и палитры), **BMP**, **PPM (`P3`/`P6`)** и базовый статический **SVG** (`viewBox`, `<rect>`, `<circle>`, `<line>`, `<polygon>`, `<path>`) с защитой от dimension-бомб (`MAX_IMAGE_PIXELS = 64_000_000`).
  - Replaced layout box для `<img>` с сохранением пропорций (aspect ratio) и fallback-блоком `alt`.
  - Минималистичный chrome в `crates/droplet-paint/src/lib.rs` (`TAB_BAR_HEIGHT = 30`, `TOOLBAR_HEIGHT = 44`, `STATUS_BAR_HEIGHT = 0` с плавающей hover-пилюлей внизу слева, SDF скруглённые прямоугольники) + **Multi-Tab `TabState` & Tab Hibernation** (`MAX_WARM_TABS = 3`, `Ctrl+T` / `Ctrl+W` / `Ctrl+Tab`) и кроссплатформенный `crates/droplet-platform/src/` (`linux_x11.rs` + `windows.rs` + `macos.rs` stub).
- **Follow-up (вынесено в `DROP-011`):** baseline **JPEG** (и опционально WebP) декодер, нативный Wayland-бэкенд и Windows Schannel TLS.
- *Спецификация:* [DROP-004-truetype-fonts-and-images.md](.scratch/issues/DROP-004-truetype-fonts-and-images.md).

---

### Phase 5 (`DROP-005`) — Forms, Selection & Interactive Controls `[DONE]`
Полноценное взаимодействие с формами и текстом на странице:
- HTML-элементы управления: `<input>` (`text`, `password`, `checkbox`, `radio`, `submit`), `<textarea>`, `<button>`, `<select>`.
- Система фокуса (`focused_node: Option<NodeId>`), навигация по `Tab` / `Shift+Tab`, визуальное кольцо фокуса (focus ring), отправка форм (`GET` / `POST` query-encoding).
- Выделение текста мышью (`SelectionRange`), копирование/вставка через платформенный буфер обмена (X11 `CLIPBOARD`/`PRIMARY` + Win32 Clipboard в `crates/droplet-platform/src/`).

---

### Phase 6 (`DROP-006`) — Persistent Bookmarks/History, Find-in-Page & DevTools `[DONE]`
*(Примечание: базовый Multi-Tab UI `TabState`, горячие клавиши `Ctrl+T`/`Ctrl+W`/`Ctrl+Tab` и Tab Hibernation уже реализованы в Phase 4 / `browser.rs`.)*  
Дальнейшее развитие daily-driver оболочки и инструментов разработчика:
- Персистентные закладки и история посещений (компактный бинарный store `crates/droplet-storage` с быстрым префиксным автодополнением в адресной строке, `about:history` / `about:bookmarks`).
- Поиск по странице (`Ctrl+F` с подсветкой совпадений и переходами `Enter`/`Shift+Enter`), быстрое переключение вкладок `Ctrl+1..9`.
- Встроенный **Box Model Inspector Overlay** (подсветка `content` / `padding` / `border` / `margin` под курсором, просмотр дерева DOM/ComputedStyle).

---

### Phase 7 (`DROP-007`, `DROP-008`) — Minimalist Scripting & Reactive DOM `[DONE]`
Поддержка базового клиентского JavaScript без монструозного JIT:
- **DROP-007 (Bytecode VM — `DONE`):** компактный парсер и стековая bytecode VM в `crates/droplet-js` (переменные, функции, замыкания, объекты, массивы, базовый event loop и таймеры).
- **DROP-008 (Reactive DOM Bindings — `DONE`):** `DomHost` + минимальные DOM API (`document.querySelector`, `getElementById`, `addEventListener`, `textContent`, `classList`, `style`), диспетчеризация событий (bubbling + `preventDefault`/`stopPropagation`) и `DirtyFlags` для инкрементального relayout/repaint.

---

### Phase 8 (`DROP-009`, `DROP-010`) — Hardware GPU Rendering & Multi-Process Sandbox `[DONE]`
Финальный уровень производительности и безопасности:
- **DROP-009 (Hardware GPU / Zero-Copy Blitting — `DONE`):** X11 `MIT-SHM`, Win32 `DIBSection` (`BitBlt`), Wayland `wl_shm` `damage_buffer`, external-slice `Canvas` и `DamageRect`-трекинг без лишних копирований буфера кадра.
- **DROP-010 (Multi-Process Sandbox — `DONE`):** разделение на привилегированный Browser/UI-процесс (`RendererHost` в `apps/droplet-desktop`) и изолированные `--renderer`-процессы (`crates/droplet-sandbox`: Linux `seccomp-bpf` + namespaces/`rlimit`, Windows Job Objects + Restricted Token, macOS `sandbox_init`; `crates/droplet-ipc`: `memfd`/shared framebuffer + бинарный IPC протокол; `HeadlessRenderer` в `crates/droplet-shell`).

---

### Phase 9 / Cross-Platform & Media Polish (`DROP-011`) — Wayland, Windows Schannel TLS & JPEG `[DONE]`
Доводка кроссплатформенного стека и недостающих форматов без добавления тяжёлых крейтов:
- **Wayland Native Client (`crates/droplet-platform/src/linux_wayland.rs`):** runtime `dlopen("libwayland-client.so.0")` + `xdg-shell` + shared-memory (`memfd_create` / `wl_shm`) с автоматическим fallback на X11 (`linux_x11.rs`), если Wayland compositor недоступен.
- **Windows Schannel TLS (`crates/droplet-net/src/lib.rs`):** нативный TLS-клиент на Windows через `secur32.dll` / `crypt32.dll` (`InitializeSecurityContextW` + проверка системного хранилища сертификатов Windows) для работы `https://` без OpenSSL.
- **Pure-Rust Baseline JPEG Decoder (`crates/droplet-image/src/lib.rs`):** декодирование baseline DCT JPEG (`SOF0`, Huffman tables, YCbCr -> RGBA8, защита `MAX_IMAGE_PIXELS = 64_000_000`) в чистом Rust без сторонних крейтов.
- **macOS Cocoa/Metal (`crates/droplet-platform/src/macos.rs`):** реализация нативного окна Cocoa + `CAMetalLayer`/`CGContext` при сборке на macOS с Apple SDK.
