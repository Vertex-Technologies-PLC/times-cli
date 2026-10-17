use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{json, Value};

use crate::cli::{Framework, HostingProvider, InitArgs, Kind, Mode};
use crate::error::CliError;
use crate::manifest::{self, bump, LOCALES, MANIFEST_FILE};
use crate::ui::Output;

struct Copy {
    title: &'static str,
    body: &'static str,
    summary: &'static str,
    highlight: &'static str,
}

fn copy(locale: &str) -> Copy {
    match locale {
        "am" => Copy {
            title: "እየሰራ ነው።",
            body: "የእርስዎን AIP ለመገንባት የመነሻ ገጹን ያርትዑ።",
            summary: "የመጀመሪያ ልቀት።",
            highlight: "መተግበሪያውን ከስራ ቦታዎ ይክፈቱ።",
        },
        "om" => Copy {
            title: "Ni hojjeta.",
            body: "AIP kee ijaaruuf fuula jalqabaa gulaali.",
            summary: "Gadhiifama jalqabaa.",
            highlight: "Appicha bakka hojii keessanii banaa.",
        },
        "ti" => Copy {
            title: "ይሰርሕ ኣሎ።",
            body: "AIP ኻ ንምህናጽ ናይ መጀመርታ ገጽ ኣርትዕ።",
            summary: "ቀዳማይ ሕታም።",
            highlight: "ነቲ መተግበሪ ካብ ቦታ ስራሕኩም ክፈቱዎ።",
        },
        _ => Copy {
            title: "It works.",
            body: "Edit the starter page to build your AIP.",
            summary: "First release.",
            highlight: "Open the app from your workspace.",
        },
    }
}

pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

fn title_case(slug: &str) -> String {
    slug.split('-')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

pub struct Options {
    pub slug: String,
    pub name: String,
    pub kind: Kind,
    pub framework: Framework,
    pub mode: Mode,
    pub hosting: HostingProvider,
    pub entry_url: Option<String>,
}

pub fn is_cwd_marker(p: &Path) -> bool {
    matches!(p.to_str(), Some("." | "./" | ".\\"))
}

pub fn resolve_project_dir(cwd: &Path, dir_arg: Option<&Path>, slug: &str) -> PathBuf {
    match dir_arg {
        None => cwd.join(slug),
        Some(d) if is_cwd_marker(d) => cwd.to_path_buf(),
        Some(d) if d.is_absolute() => d.to_path_buf(),
        Some(d) => cwd.join(d),
    }
}

fn folder_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty() && s != "." && s != "..")
        .unwrap_or_else(|| "my-aip".into())
}

pub fn run(out: &Output, args: InitArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let in_place = args.dir.as_ref().is_some_and(|d| is_cwd_marker(d));

    let kind = match args.kind {
        Some(k) => k,
        None if out.interactive => {
            let opts = vec!["app".to_string(), "integration".into(), "pack".into()];
            [Kind::App, Kind::Integration, Kind::Pack][out.select("Kind", opts)?]
        }
        None => Kind::App,
    };
    let framework = match args.framework {
        Some(f) => f,
        None if out.interactive => {
            let opts = vec!["vite".to_string(), "static".into(), "next".into()];
            [Framework::Vite, Framework::Static, Framework::Next][out.select("Framework", opts)?]
        }
        None => Framework::Vite,
    };
    let mode = args.mode.unwrap_or(Mode::Platform);
    let hosting = resolve_hosting(out, framework, args.hosting)?;
    let entry_url = match hosting {
        HostingProvider::Developer => {
            let default = args
                .entry_url
                .clone()
                .unwrap_or_else(|| "https://your-app.example.com/".into());
            let url = if args.entry_url.is_some() {
                default
            } else if out.interactive {
                out.text("Public HTTPS origin", &default)?
            } else {
                default
            };
            if !manifest::is_public_https(&url) {
                return Err(CliError::usage(
                    "hosting.entryUrl must be a public https:// URL (no localhost).",
                ));
            }
            Some(url)
        }
        HostingProvider::Times => None,
    };

    let default_slug = {
        let raw = if in_place {
            folder_name(&cwd)
        } else if let Some(d) = &args.dir {
            folder_name(d)
        } else {
            "my-aip".into()
        };
        let s = slugify(&raw);
        if s.is_empty() {
            "my-aip".into()
        } else {
            s
        }
    };
    let slug = match args.slug {
        Some(s) => s,
        None if out.interactive => out.text("Slug", &default_slug)?,
        None => default_slug,
    };
    if !is_valid_slug(&slug) {
        return Err(CliError::usage(format!(
            "`{slug}` is not a valid slug. Use lowercase letters, digits and single hyphens, like invoice-reminders."
        )));
    }
    let dir = resolve_project_dir(&cwd, args.dir.as_deref(), &slug);
    let name = match args.name {
        Some(n) => n,
        None if out.interactive => out.text("Name", &title_case(&slug))?,
        None => title_case(&slug),
    };

    let opts = Options {
        slug,
        name,
        kind,
        framework,
        mode,
        hosting,
        entry_url,
    };
    let files = scaffold(&opts);

    let conflicts: Vec<&String> = files
        .iter()
        .map(|(p, _)| p)
        .filter(|p| dir.join(p).exists())
        .collect();
    if !conflicts.is_empty() && !args.force {
        return Err(CliError::usage(format!(
            "These files already exist in {}: {}. Pass --force to overwrite them.",
            dir.display(),
            conflicts
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    for (rel, content) in &files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content).with_context(|| format!("Could not write {}", path.display()))?;
    }

    let manifest_value: Value = serde_json::from_str(&files[0].1)?;
    let report = manifest::validate(&manifest_value, None);
    debug_assert!(report.valid, "{:?}", report.errors);

    if out.json {
        return out.json(&json!({
            "dir": dir,
            "slug": opts.slug,
            "kind": opts.kind.as_str(),
            "framework": opts.framework.as_str(),
            "mode": opts.mode.as_str(),
            "hosting": opts.hosting.as_str(),
            "files": files.iter().map(|(p, _)| p).collect::<Vec<_>>(),
        }));
    }

    out.success(format!(
        "Created {} ({} {}) in {}",
        opts.slug,
        opts.framework.as_str(),
        opts.kind.as_str(),
        display_dir(&cwd, &dir)
    ));
    if opts.mode == Mode::Runtime {
        out.warn("Set runtime.workerUrl in times.aip.json to your worker's public https URL.");
    }
    out.info("");
    out.info("Next steps:");
    if dir != cwd {
        out.info(format!("  cd {}", display_dir(&cwd, &dir)));
    }
    match opts.hosting {
        HostingProvider::Developer => {
            out.info("  bun install");
            out.info("  bun run dev");
            out.info("Set hosting.entryUrl to your production HTTPS origin before `times aip push`.");
            out.info("  times aip origin check");
            out.info("  times aip push");
        }
        HostingProvider::Times => {
            match opts.framework {
                Framework::Static => {}
                _ => {
                    out.info("  bun install");
                    out.info("  bun run build");
                }
            }
            out.info("  times dev");
            out.info("  times aip push");
        }
    }
    Ok(())
}

fn resolve_hosting(
    out: &Output,
    framework: Framework,
    explicit: Option<HostingProvider>,
) -> anyhow::Result<HostingProvider> {
    if let Some(h) = explicit {
        return Ok(h);
    }
    if framework != Framework::Next {
        return Ok(HostingProvider::Times);
    }
    if !out.interactive {
        return Ok(HostingProvider::Times);
    }
    let opts = vec![
        "TIMES static export".to_string(),
        "I host it (public HTTPS)".into(),
    ];
    Ok([HostingProvider::Times, HostingProvider::Developer]
        [out.select("Hosting", opts)?])
}

fn display_dir(cwd: &Path, dir: &Path) -> String {
    dir.strip_prefix(cwd)
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.to_path_buf())
        .display()
        .to_string()
}

fn entry_for(o: &Options) -> String {
    if o.hosting == HostingProvider::Developer {
        return o
            .entry_url
            .clone()
            .unwrap_or_else(|| "https://your-app.example.com/".into());
    }
    match o.framework {
        Framework::Vite => "./dist/index.html".into(),
        Framework::Next => "./out/index.html".into(),
        Framework::Static => "./index.html".into(),
    }
}

pub fn manifest_json(o: &Options) -> Value {
    let surfaces = match o.kind {
        Kind::Integration => json!(["configure"]),
        _ => json!(["page"]),
    };
    let prefix = bump::changelog_prefix("1.0.0");
    let mut m = json!({
        "schemaVersion": "1",
        "slug": o.slug,
        "kind": o.kind.as_str(),
        "version": "1.0.0",
        "name": o.name,
        "mode": o.mode.as_str(),
        "hosting": hosting_json(o),
        "ui": {
            "framework": o.framework.as_str(),
            "runtime": "iframe",
            "surfaces": surfaces,
            "entry": entry_for(o),
            "icon": "./icon.svg",
            "uses_times_tokens": true
        },
        "scopes": { "required": ["company:read"] },
        "i18n": {
            "locales": LOCALES,
            "catalogs": {
                "en": "./i18n/messages/en.json",
                "am": "./i18n/messages/am.json",
                "om": "./i18n/messages/om.json",
                "ti": "./i18n/messages/ti.json"
            }
        },
        "changelog": [{
            "version": "1.0.0",
            "date": bump::today(),
            "summaryKey": format!("{prefix}.summary"),
            "highlightKeys": [format!("{prefix}.h1")]
        }]
    });
    if o.mode == Mode::Runtime {
        m["runtime"] = json!({
            "workerUrl": "https://your-worker.example.com",
            "webhookPath": "/times/webhook",
            "signing": { "algorithm": "hmac-sha256", "header": "X-Times-Signature", "skewSeconds": 300 }
        });
    }
    m
}

fn hosting_json(o: &Options) -> Value {
    match o.hosting {
        HostingProvider::Developer => json!({
            "provider": "developer",
            "entryUrl": o.entry_url.as_deref().unwrap_or("https://your-app.example.com/")
        }),
        HostingProvider::Times => json!({ "provider": "times" }),
    }
}

fn catalog(locale: &str, name: &str) -> String {
    let c = copy(locale);
    let v = json!({
        "meta": { "name": name },
        "home": { "title": c.title, "body": c.body },
        "changelog": { "v1_0_0": { "summary": c.summary, "h1": c.highlight } }
    });
    pretty(&v)
}

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).expect("json");
    s.push('\n');
    s
}

const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#2563eb"/><path d="M20 22h24v6H35v16h-6V28h-9z" fill="#fff"/></svg>
"##;

const GITIGNORE: &str = "node_modules\ndist\nout\n.next\n*.zip\n.DS_Store\n.times/\n";

const MAIN_TS: &str = r#"import '@times/aip-sdk/tokens.css';
import './style.css';
import { createTimesAipClient } from '@times/aip-sdk';

const client = createTimesAipClient({
  parentOrigin: 'https://app.times.et',
  apiBaseUrl: 'https://api.times.et'
});

const app = document.querySelector<HTMLDivElement>('#app')!;

client.waitForInit().then(() => {
  const t = client.t;
  client.setTitle(t('meta.name'));
  const heading = document.createElement('h1');
  heading.textContent = t('home.title');
  const body = document.createElement('p');
  body.textContent = t('home.body');
  app.replaceChildren(heading, body);
  client.ready('1.0.0');
});
"#;

const STYLE_CSS: &str = r#"body {
  margin: 0;
  padding: 24px;
  font-family: var(--font-sans, system-ui, sans-serif);
  background: var(--background);
  color: var(--foreground);
}

h1 {
  margin: 0 0 8px;
  font-size: 1.25rem;
}

p {
  margin: 0;
  color: var(--muted-foreground);
}
"#;

const VITE_CONFIG: &str = r#"import { defineConfig } from 'vite';

export default defineConfig({
  base: './',
  build: { outDir: 'dist' }
});
"#;

const TSCONFIG_VITE: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "lib": ["ES2022", "DOM"],
    "strict": true,
    "skipLibCheck": true,
    "noEmit": true
  },
  "include": ["src"]
}
"#;

const STATIC_JS: &str = r#"const PARENT_ORIGIN = 'https://app.times.et';

function lookup(messages, key) {
  return key.split('.').reduce((node, part) => (node ? node[part] : undefined), messages) ?? key;
}

window.addEventListener('message', (event) => {
  if (event.origin !== PARENT_ORIGIN) return;
  const msg = event.data;
  if (!msg || msg.type !== 'init') return;
  const messages = msg.payload?.messages ?? {};
  document.getElementById('title').textContent = lookup(messages, 'home.title');
  document.getElementById('body').textContent = lookup(messages, 'home.body');
  window.parent.postMessage({ v: 1, type: 'ready', payload: { version: 1, appVersion: '1.0.0' } }, PARENT_ORIGIN);
});
"#;

const NEXT_CONFIG: &str = r#"/** @type {import('next').NextConfig} */
const nextConfig = {
  output: 'export',
  assetPrefix: './',
  images: { unoptimized: true }
};

export default nextConfig;
"#;

const NEXT_CONFIG_DEVELOPER: &str = r#"/** @type {import('next').NextConfig} */
const nextConfig = {
  images: { unoptimized: true },
  async headers() {
    return [
      {
        source: '/:path*',
        headers: [
          {
            key: 'Content-Security-Policy',
            value: 'frame-ancestors https://app.times.et https://*.times.et'
          }
        ]
      }
    ];
  }
};

export default nextConfig;
"#;

const NEXT_LAYOUT: &str = r#"import '@times/aip-sdk/tokens.css';
import type { ReactNode } from 'react';

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang='en'>
      <body>{children}</body>
    </html>
  );
}
"#;

const NEXT_PAGE: &str = r#"'use client';

import { useEffect, useState } from 'react';
import { createTimesAipClient, type TimesAipClient } from '@times/aip-sdk';

export default function Home() {
  const [client, setClient] = useState<TimesAipClient | null>(null);

  useEffect(() => {
    const c = createTimesAipClient({
      parentOrigin: 'https://app.times.et',
      apiBaseUrl: 'https://api.times.et'
    });
    c.waitForInit().then(() => {
      c.setTitle(c.t('meta.name'));
      setClient(c);
      c.ready('1.0.0');
    });
  }, []);

  if (!client) return null;
  return (
    <main style={{ padding: 24 }}>
      <h1>{client.t('home.title')}</h1>
      <p>{client.t('home.body')}</p>
    </main>
  );
}
"#;

const TSCONFIG_NEXT: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["dom", "dom.iterable", "esnext"],
    "allowJs": false,
    "skipLibCheck": true,
    "strict": true,
    "noEmit": true,
    "esModuleInterop": true,
    "module": "esnext",
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "jsx": "preserve",
    "incremental": true,
    "plugins": [{ "name": "next" }]
  },
  "include": ["next-env.d.ts", "**/*.ts", "**/*.tsx"],
  "exclude": ["node_modules"]
}
"#;

pub fn scaffold(o: &Options) -> Vec<(String, String)> {
    let mut files = vec![
        (MANIFEST_FILE.to_string(), pretty(&manifest_json(o))),
        ("icon.svg".into(), ICON.into()),
        (".gitignore".into(), GITIGNORE.into()),
    ];
    for l in LOCALES {
        files.push((format!("i18n/messages/{l}.json"), catalog(l, &o.name)));
    }

    match o.framework {
        Framework::Vite => {
            let pkg = json!({
                "name": o.slug,
                "private": true,
                "version": "1.0.0",
                "type": "module",
                "scripts": { "dev": "vite", "build": "tsc && vite build", "preview": "vite preview" },
                "dependencies": { "@times/aip-sdk": "^1.0.0" },
                "devDependencies": { "typescript": "^5.7.0", "vite": "^6.0.0" }
            });
            files.push(("package.json".into(), pretty(&pkg)));
            files.push(("vite.config.ts".into(), VITE_CONFIG.into()));
            files.push(("tsconfig.json".into(), TSCONFIG_VITE.into()));
            files.push((
                "index.html".into(),
                format!(
                    "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <title>{}</title>\n  </head>\n  <body>\n    <div id=\"app\"></div>\n    <script type=\"module\" src=\"/src/main.ts\"></script>\n  </body>\n</html>\n",
                    html_escape(&o.name)
                ),
            ));
            files.push(("src/main.ts".into(), MAIN_TS.into()));
            files.push(("src/style.css".into(), STYLE_CSS.into()));
        }
        Framework::Static => {
            files.push((
                "index.html".into(),
                format!(
                    "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <title>{}</title>\n    <link rel=\"stylesheet\" href=\"./style.css\" />\n  </head>\n  <body>\n    <h1 id=\"title\"></h1>\n    <p id=\"body\"></p>\n    <script src=\"./app.js\"></script>\n  </body>\n</html>\n",
                    html_escape(&o.name)
                ),
            ));
            files.push(("app.js".into(), STATIC_JS.into()));
            files.push(("style.css".into(), STYLE_CSS.into()));
        }
        Framework::Next => {
            let pkg = json!({
                "name": o.slug,
                "private": true,
                "version": "1.0.0",
                "scripts": { "dev": "next dev", "build": "next build" },
                "dependencies": {
                    "@times/aip-sdk": "^1.0.0",
                    "next": "^16.0.0",
                    "react": "^19.0.0",
                    "react-dom": "^19.0.0"
                },
                "devDependencies": {
                    "@types/node": "^22.0.0",
                    "@types/react": "^19.0.0",
                    "typescript": "^5.7.0"
                }
            });
            files.push(("package.json".into(), pretty(&pkg)));
            files.push((
                "next.config.mjs".into(),
                if o.hosting == HostingProvider::Developer {
                    NEXT_CONFIG_DEVELOPER.into()
                } else {
                    NEXT_CONFIG.into()
                },
            ));
            files.push(("tsconfig.json".into(), TSCONFIG_NEXT.into()));
            files.push(("app/layout.tsx".into(), NEXT_LAYOUT.into()));
            files.push(("app/page.tsx".into(), NEXT_PAGE.into()));
        }
    }
    files
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(framework: Framework, kind: Kind, mode: Mode) -> Options {
        Options {
            slug: "invoice-reminders".into(),
            name: "Invoice Reminders".into(),
            kind,
            framework,
            mode,
            hosting: HostingProvider::Times,
            entry_url: None,
        }
    }

    #[test]
    fn resolve_dir_uses_slug_unless_dot() {
        let cwd = Path::new("/work/integrations");
        assert_eq!(
            resolve_project_dir(cwd, None, "times-peachtree"),
            PathBuf::from("/work/integrations/times-peachtree")
        );
        assert_eq!(
            resolve_project_dir(cwd, Some(Path::new(".")), "times-peachtree"),
            PathBuf::from("/work/integrations")
        );
        assert_eq!(
            resolve_project_dir(cwd, Some(Path::new("custom")), "times-peachtree"),
            PathBuf::from("/work/integrations/custom")
        );
        assert!(is_cwd_marker(Path::new(".")));
        assert!(!is_cwd_marker(Path::new("times-peachtree")));
    }

    #[test]
    fn slug_helpers() {
        assert_eq!(slugify("My First App!"), "my-first-app");
        assert_eq!(slugify("--x__y--"), "x-y");
        assert_eq!(title_case("invoice-reminders"), "Invoice Reminders");
        assert!(is_valid_slug("a-1"));
        assert!(!is_valid_slug("A"));
        assert!(!is_valid_slug("a--b"));
        assert!(!is_valid_slug("-a"));
    }

    #[test]
    fn every_scaffold_is_valid_on_disk() {
        for fw in [Framework::Vite, Framework::Static, Framework::Next] {
            for kind in [Kind::App, Kind::Integration, Kind::Pack] {
                for mode in [Mode::Platform, Mode::Runtime] {
                    let dir = tempfile::tempdir().unwrap();
                    for (rel, content) in scaffold(&opts(fw, kind, mode)) {
                        let p = dir.path().join(rel);
                        fs::create_dir_all(p.parent().unwrap()).unwrap();
                        fs::write(p, content).unwrap();
                    }
                    let m = manifest::load(dir.path()).unwrap();
                    let r = manifest::validate(&m.value, Some(dir.path()));
                    assert!(r.valid, "{fw:?} {kind:?} {mode:?}: {:?}", r.errors);
                    let only_entry_warning = r.warnings.iter().all(|w| w.path == "/ui/entry");
                    assert!(only_entry_warning, "{:?}", r.warnings);
                }
            }
        }
    }

    #[test]
    fn developer_next_uses_origin_and_csp() {
        let o = Options {
            slug: "peachtree".into(),
            name: "Peachtree".into(),
            kind: Kind::App,
            framework: Framework::Next,
            mode: Mode::Platform,
            hosting: HostingProvider::Developer,
            entry_url: Some("https://peachtree.example.com/".into()),
        };
        let files: std::collections::BTreeMap<_, _> = scaffold(&o).into_iter().collect();
        let m: Value = serde_json::from_str(&files["times.aip.json"]).unwrap();
        assert_eq!(m["hosting"]["provider"], "developer");
        assert_eq!(m["hosting"]["entryUrl"], "https://peachtree.example.com/");
        assert_eq!(m["ui"]["entry"], "https://peachtree.example.com/");
        assert!(files["next.config.mjs"].contains("frame-ancestors"));
        assert!(!files["next.config.mjs"].contains("output: 'export'"));
        let r = manifest::validate(&m, None);
        assert!(r.valid, "{:?}", r.errors);
    }

    #[test]
    fn catalogs_are_translated() {
        let en = catalog("en", "X");
        for l in ["am", "om", "ti"] {
            let other = catalog(l, "X");
            assert_ne!(en, other, "{l} must not copy English");
        }
    }
}
