use droplet_storage::Store;
use droplet_url::Url;

use crate::Response;

pub(crate) fn fetch_about(url: &Url) -> Response {
    let (status, body) = match url.path.as_str() {
        "droplet" | "home" => (200, about_droplet_html()),
        "roadmap" => (200, about_roadmap_html()),
        "architecture" => (200, about_architecture_html()),
        "test" => (200, about_test_html()),
        "history" => {
            let store = Store::from_env();
            (200, crate::user_pages::history_page(&store.recent_history(200)))
        }
        "bookmarks" => {
            let store = Store::from_env();
            let bookmarks = store.bookmarks();
            let recent: Vec<_> = bookmarks.iter().rev().take(200).cloned().collect();
            (200, crate::user_pages::bookmarks_page(&recent))
        }
        "blank" => (200, about_blank_html()),
        other => (404, about_not_found_html(other)),
    };

    Response {
        url: url.clone(),
        status,
        content_type: "text/html; charset=utf-8".into(),
        body,
    }
}

pub(crate) const ABOUT_BASE_CSS: &str = r#"
body {
  background-color: #f4f7fb;
  color: #162032;
  font-size: 16px;
  margin: 0;
  padding: 20px;
}
.shell {
  background-color: #ffffff;
  border: 1px solid #cbd5e1;
  padding: 24px;
  margin-bottom: 16px;
}
.nav {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
  background-color: #0f172a;
  color: #e2e8f0;
  padding: 12px 18px;
  margin-bottom: 18px;
}
.nav-links {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.nav a {
  display: inline-block;
  color: #7dd3fc;
  text-decoration: none;
  font-weight: bold;
  padding: 4px 10px;
  background-color: #1e293b;
  border: 1px solid #334155;
}
.nav a:hover {
  background-color: #2563eb;
  color: #ffffff;
  border: 1px solid #93c5fd;
}
.nav a:active {
  background-color: #1d4ed8;
  color: #ffffff;
}
.brand {
  color: #ffffff;
  font-weight: bold;
  font-size: 17px;
}
h1 {
  color: #0f172a;
  font-size: 26px;
  margin-top: 0;
  margin-bottom: 10px;
}
h2 {
  color: #1e3a8a;
  font-size: 20px;
  margin-top: 18px;
  margin-bottom: 8px;
}
h3 {
  color: #0f172a;
  font-size: 17px;
  margin-top: 12px;
  margin-bottom: 6px;
}
p {
  margin-top: 0;
  margin-bottom: 12px;
}
a {
  color: #1d4ed8;
}
a:hover {
  color: #1e40af;
  background-color: #dbeafe;
}
.flex-row {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: 12px;
  margin-bottom: 14px;
}
.flex-card {
  flex-grow: 1;
  flex-basis: 240px;
  background-color: #f8fafc;
  border: 1px solid #cbd5e1;
  padding: 14px 16px;
}
.flex-card:hover {
  background-color: #eff6ff;
  border: 1px solid #60a5fa;
}
.btn {
  display: inline-block;
  background-color: #1d4ed8;
  color: #ffffff;
  text-decoration: none;
  font-weight: bold;
  padding: 7px 14px;
  border: 1px solid #1e40af;
  margin-right: 8px;
  margin-bottom: 6px;
}
.btn:hover {
  background-color: #2563eb;
  color: #ffffff;
}
.btn:active {
  background-color: #1e3a8a;
  color: #ffffff;
}
.badge {
  display: inline-block;
  background-color: #dbeafe;
  color: #1e3a8a;
  border: 1px solid #93c5fd;
  padding: 2px 8px;
  font-size: 13px;
  font-weight: bold;
}
.card {
  background-color: #f8fafc;
  border: 1px solid #cbd5e1;
  padding: 14px 16px;
  margin-bottom: 12px;
}
.accent {
  background-color: #eff6ff;
  border: 1px solid #93c5fd;
  padding: 14px 16px;
  margin-bottom: 14px;
}
.done {
  background-color: #ecfdf5;
  border: 1px solid #6ee7b7;
  padding: 14px 16px;
  margin-bottom: 12px;
}
.meta {
  color: #475569;
  font-size: 14px;
}
pre {
  background-color: #0f172a;
  color: #e2e8f0;
  padding: 12px 14px;
  border: 1px solid #334155;
  margin-bottom: 12px;
}
code {
  background-color: #e2e8f0;
  color: #0f172a;
  padding: 2px 4px;
}
pre code {
  background-color: #0f172a;
  color: #e2e8f0;
  padding: 0;
}
ul {
  margin-top: 4px;
  margin-bottom: 12px;
  padding-left: 22px;
}
li {
  margin-bottom: 5px;
}
hr {
  border: 1px solid #cbd5e1;
  margin-top: 14px;
  margin-bottom: 14px;
}
"#;

pub(crate) fn about_nav_html(active: &str) -> String {
    format!(
        "<div class=\"nav\">\
         <span class=\"brand\">Droplet Engine</span>\
         <div class=\"nav-links\">\
         <a href=\"about:droplet\">Home{h}</a>\
         <a href=\"about:roadmap\">Roadmap{r}</a>\
         <a href=\"about:architecture\">Architecture{a}</a>\
         <a href=\"about:test\">Visual Test{t}</a>\
         <a href=\"about:history\">History{hi}</a>\
         <a href=\"about:bookmarks\">Bookmarks{b}</a>\
         <a href=\"about:blank\">Blank</a>\
         </div>\
         </div>",
        h = if active == "droplet" { " *" } else { "" },
        r = if active == "roadmap" { " *" } else { "" },
        a = if active == "architecture" { " *" } else { "" },
        t = if active == "test" { " *" } else { "" },
        hi = if active == "history" { " *" } else { "" },
        b = if active == "bookmarks" { " *" } else { "" },
    )
}

fn about_droplet_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Fast Browser Engine</title>
  <style>{ABOUT_BASE_CSS}</style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Droplet Browser Engine</h1>
    <p class="meta">Zero-bloat graphical web browser written from scratch in pure Rust (&laquo;очень быстрый браузер без мусора&raquo;).</p>

    <div class="accent">
      <h3>Why Droplet? / Зачем Droplet?</h3>
      <p>Modern browsers ship tens of millions of lines of legacy code and eat gigabytes of RAM. Droplet targets sub-20ms cold startup, &lt; 15 MB idle RSS, and a crystal-clear pipeline built for speed.</p>
      <p>Цель Droplet &mdash; минимальный вес, мгновенный старт и чистая модульная архитектура без лишних зависимостей.</p>
    </div>

    <div class="flex-row">
      <div class="flex-card">
        <h3>Zero Bloat Core <span class="badge">Pure Rust</span></h3>
        <p>Zero Cargo dependency bloat. Dynamic <code>dlopen</code> for <code>libX11</code>, <code>libssl</code>, and system fonts keeps binary size tiny and builds sub-second.</p>
      </div>
      <div class="flex-card">
        <h3>Flexbox + :hover <span class="badge">Phase 2</span></h3>
        <p>Full <code>display: flex</code>, <code>inline-block</code>, relative units (<code>%</code>, <code>em</code>, <code>rem</code>, <code>vh</code>, <code>vw</code>), and reactive <code>:hover</code> / <code>:active</code> states.</p>
      </div>
      <div class="flex-card">
        <h3>HTTPS + Keep-Alive <span class="badge">Phase 3</span></h3>
        <p>Verified OpenSSL TLS, Keep-Alive connection pool, bounded 4-thread loader, 60s DNS cache, <code>data:</code> URIs, and zero-alloc tracker blocker.</p>
      </div>
    </div>

    <div class="card">
      <h2>Explore Built-In Pages</h2>
      <p>Hover and click any button below to test interactive <code>:hover</code> / <code>:active</code> highlights and Flexbox wrapping:</p>
      <div>
        <a class="btn" href="about:roadmap">Roadmap (Linux + Windows) &rarr;</a>
        <a class="btn" href="about:architecture">Low-RAM Architecture &rarr;</a>
        <a class="btn" href="about:test">Layout, Flexbox &amp; Cyrillic Suite &rarr;</a>
      </div>
    </div>

    <div class="card">
      <h2>Sample External HTTP &amp; HTTPS Links</h2>
      <p>Phase 3 enables verified <code>https://</code> via dynamic OpenSSL alongside <code>http://</code>, <code>file://</code>, <code>data:</code>, and <code>about:*</code>:</p>
      <ul>
        <li><a href="https://example.com/">https://example.com/ (TLS + HTTP/1.1 Keep-Alive)</a></li>
        <li><a href="http://example.com/">http://example.com/</a></li>
        <li><a href="http://info.cern.ch/">http://info.cern.ch/ (First website on the Web)</a></li>
      </ul>
    </div>
  </div>
</body>
</html>"#,
        nav = about_nav_html("droplet")
    )
}

fn about_roadmap_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Roadmap</title>
  <style>{ABOUT_BASE_CSS}</style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Droplet Development Roadmap (Linux + Windows)</h1>
    <p><a href="about:droplet">&larr; Back to about:droplet</a></p>

    <div class="card">
      <h2>Re-baseline (2026-11): depth before breadth</h2>
      <p>&laquo;Done&raquo; now means: the corpus page looks right on a screenshot <b>and</b> <code>droplet --bench</code> did not regress. Corpus score at re-baseline: <b>4 / 12</b> pages correct.</p>
    </div>

    <div class="done">
      <h2>Phase 0 (Done) &mdash; Hygiene &amp; Measurement</h2>
      <ul>
        <li>Dead legacy tree removed; JavaScript off by default (<code>--js</code>); sandbox behind cargo feature <code>sandbox</code>.</li>
        <li><code>droplet --bench</code>: median per stage (html, css, style, layout, display list, raster) + peak RSS.</li>
        <li><code>tests/corpus</code>: layout goldens with built-in font metrics and a human-reviewed <code>ok</code>/<code>wrong</code> manifest.</li>
      </ul>
    </div>

    <div class="card">
      <h2>Phase 1 &mdash; Network that never freezes the window</h2>
      <ul>
        <li>Background navigation thread, cancel, progressive paint; reliable TLS handshake; gzip/deflate; cookies.</li>
      </ul>
    </div>

    <div class="card">
      <h2>Phase 2 &mdash; Correct CSS</h2>
      <ul>
        <li>Rule index (fast style on big pages), <code>!important</code>, <code>inherit</code>/<code>initial</code>/<code>unset</code>, <code>var()</code>, <code>@media</code>, <code>@import</code>.</li>
        <li>Selectors <code>:not</code>, <code>:nth-*</code>, attributes, combinators; <code>box-sizing</code>, <code>min</code>/<code>max</code>, shorthands.</li>
      </ul>
    </div>

    <div class="card">
      <h2>Phase 3 &mdash; Layout core</h2>
      <ul>
        <li><code>position</code> + <code>z-index</code>, floats, tables, <code>overflow</code> and scroll containers, margin collapsing, line breaking.</li>
      </ul>
    </div>

    <div class="card">
      <h2>Phases 4&ndash;9</h2>
      <ul>
        <li><b>Phase 4</b>: italic &amp; font fallback, WebP, image memory budget and lazy decode.</li>
        <li><b>Phase 5</b>: scrolling without relayout, incremental restyle/relayout, enforced perf budgets.</li>
        <li><b>Phase 6</b>: daily-driver shell &mdash; forms, downloads, session restore.</li>
        <li><b>Phase 7</b>: JavaScript decision record (no-JS by design, or a real ES2020+ engine).</li>
        <li><b>Phase 8</b>: sandbox wired into the GUI, parser fuzzing, <code>unsafe</code> audit.</li>
        <li><b>Phase 9</b>: Windows and Wayland parity; macOS with a macOS host.</li>
      </ul>
    </div>
  </div>
</body>
</html>"#,
        nav = about_nav_html("roadmap")
    )
}

fn about_architecture_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Architecture</title>
  <style>{ABOUT_BASE_CSS}</style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Droplet Low-RAM Architecture</h1>
    <p><a href="about:droplet">&larr; Back to about:droplet</a></p>

    <div class="accent">
      <h2>Rendering Pipeline</h2>
      <pre><code>URL (droplet-url) -&gt; TrackerFilter -&gt; DNS Cache + Keep-Alive / TLS Pool (droplet-net)
    -&gt; Concurrent Loader -&gt; HTML Parser -&gt; Arena DOM
    -&gt; CSS + Style Tree -&gt; Block / Inline / Flexbox Layout + Images (PNG/JPEG/BMP/PPM/SVG)
    -&gt; Flat Display List -&gt; Software Rasterizer + TTF GlyphAtlas -&gt; X11 / Win32 / PPM</code></pre>
    </div>

    <div class="flex-row">
      <div class="flex-card">
        <h3>Arena DOM <span class="badge">Vec&lt;Node&gt;</span></h3>
        <p>Nodes live contiguously in one vector addressed by integer <code>NodeId(usize)</code>. Zero <code>Rc&lt;RefCell&gt;</code> overhead and iterative traversals prevent stack overflow.</p>
      </div>
      <div class="flex-card">
        <h3>Bounded Caches <span class="badge">&lt; 15 MB RSS</span></h3>
        <p>DNS cache capped at 256 entries (60s TTL), connection pool capped at 32 sockets (30s idle TTL), and reusable framebuffers keep memory flat.</p>
      </div>
      <div class="flex-card">
        <h3>Dirty Repainting <span class="badge">~0% Idle CPU</span></h3>
        <p>Scrolling and hover transitions cull off-screen <code>DisplayCommand</code>s and only re-style or re-rasterize when dirty flags change.</p>
      </div>
    </div>

    <div class="card">
      <h2>Cargo Workspace &amp; Zero-Overhead Tracker Blocker (droplet-net::adblock)</h2>
      <p>Droplet separates each stage into focused, zero-external-dependency workspace crates (including <code>droplet-url</code>, <code>droplet-net</code>, pure-Rust image decoding with PNG/JPEG/BMP/PPM/SVG, DOM/CSS/Layout, Font/Paint, and OS platform backends).</p>
      <p><code>droplet-net::adblock</code> uses a sorted static domain table with ASCII case-insensitive binary search over dot-separated suffixes &mdash; zero heap allocations per URL check, blocking trackers before DNS or TCP connect.</p>
    </div>
  </div>
</body>
</html>"#,
        nav = about_nav_html("architecture")
    )
}

fn about_test_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Visual Test Suite</title>
  <style>{ABOUT_BASE_CSS}
  .box-a {{
    background-color: #fef3c7;
    border: 2px solid #d97706;
    padding: 12px 16px;
    margin-bottom: 12px;
  }}
  .box-b {{
    background-color: #e0f2fe;
    border: 1px solid #0284c7;
    padding: 10px 14px;
    margin: 8px 0 14px 16px;
  }}
  .demo-flex {{
    display: flex;
    flex-direction: row;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    background-color: #f1f5f9;
    border: 1px solid #cbd5e1;
    padding: 12px;
    margin-bottom: 14px;
  }}
  .demo-item {{
    flex-grow: 1;
    background-color: #ffffff;
    border: 1px solid #94a3b8;
    padding: 10px;
  }}
  .demo-item:hover {{
    background-color: #dbeafe;
    border: 1px solid #2563eb;
  }}
  </style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Heading H1 &mdash; Тест рендеринга</h1>
    <h2>Heading H2 &mdash; Flexbox, :hover &amp; Box Model</h2>
    <h3>Heading H3 &mdash; Кириллица и ASCII</h3>

    <p>This paragraph tests inline text flow with <a href="about:droplet">clickable links (about:droplet)</a> embedded inside wrapping prose, alongside Russian Cyrillic text: Быстрая капля бежит по экрану без лишнего мусора и тормозов.</p>

    <div class="demo-flex">
      <div class="demo-item"><b>Flex Card 1</b>: hover highlight test</div>
      <div class="demo-item"><b>Flex Card 2</b>: equal <code>flex-grow: 1</code></div>
      <div class="demo-item"><b>Flex Card 3</b>: <code>gap: 12px</code></div>
    </div>

    <div class="box-a">
      <b>Box A</b>: 2px amber border, 12px/16px padding, 12px bottom margin.
      <div class="box-b">
        <b>Nested Box B</b>: 1px blue border with left margin offset and <a href="about:roadmap">link to Roadmap</a>.
      </div>
    </div>

    <hr>

    <h3>List &amp; Code Block</h3>
    <ul>
      <li>Элемент списка один &bull; Entity check: &laquo;Droplet&raquo; &ndash; fast &amp; tiny</li>
      <li>Элемент списка два &bull; Arrows: &larr; &rarr; &bull; Quotes: &ldquo;ok&rdquo;</li>
      <li>Nested link inside list: <a href="about:architecture">Open Architecture</a></li>
    </ul>

    <pre><code>fn hello_droplet() {{
    println!("Привет, Droplet! 2 &lt; 5 &amp;&amp; 5 &gt; 2");
}}</code></pre>
  </div>
</body>
</html>"#,
        nav = about_nav_html("test")
    )
}

fn about_blank_html() -> String {
    "<!DOCTYPE html><html><head><title>about:blank</title></head><body></body></html>".into()
}

fn about_not_found_html(page: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Unknown Page</title>
  <style>{ABOUT_BASE_CSS}</style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Unknown built-in page: about:{page}</h1>
    <p>Available built-in pages:</p>
    <ul>
      <li><a href="about:droplet">about:droplet</a></li>
      <li><a href="about:roadmap">about:roadmap</a></li>
      <li><a href="about:architecture">about:architecture</a></li>
      <li><a href="about:test">about:test</a></li>
      <li><a href="about:history">about:history</a></li>
      <li><a href="about:bookmarks">about:bookmarks</a></li>
      <li><a href="about:blank">about:blank</a></li>
    </ul>
  </div>
</body>
</html>"#,
        nav = about_nav_html("")
    )
}
