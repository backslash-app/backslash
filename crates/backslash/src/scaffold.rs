//! `backslash new <id>`: a working extension in one command.

use std::path::Path;

use anyhow::{bail, Result};
use backslash_core::paths;

/// `python`: an extension using the Python SDK instead of TypeScript.
pub fn create(id: &str, parent: Option<&Path>, python: bool) -> Result<()> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        bail!("extension id must match [a-z0-9-]+");
    }
    let dir = parent.unwrap_or(Path::new(".")).join(id);
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    std::fs::create_dir_all(dir.join("commands"))?;

    let title = title_case(id);
    std::fs::write(
        dir.join("backslash.toml"),
        format!(
            r#"[extension]
id = "{id}"
title = "{title}"
description = "Describe what {title} does"
version = "0.1.0"
author = ""
icon = "applications-other"
license = "MIT"

{runtime}# Everything not listed here is denied. Examples:
#   net = ["api.github.com"]   run = ["git"]   read = ["~/Documents"]
[permissions]

[[commands]]
name = "hello"
title = "Hello from {title}"
keywords = ["{id}"]
"#,
            runtime = if python { "[runtime]\nkind = \"python\"\n\n" } else { "" },
        ),
    )?;

    if python {
        std::fs::write(dir.join("commands/hello.py"), PYTHON_HELLO)?;
        std::fs::write(dir.join(".gitignore"), "__pycache__\n")?;
        return done(&dir, id);
    }

    std::fs::write(
        dir.join("commands/hello.ts"),
        r##"import { action, close, copy, detail, list, toast } from "@backslash/api";

export default list({
  placeholder: "Search greetings…",
  items: [
    {
      title: "Hello, Linux",
      subtitle: "Press Enter to see details",
      icon: "👋",
      tags: ["new"],
      actions: [
        action("Show Details", () => detail("# Hello!\n\nEdit `commands/hello.ts`, then reopen this command.")),
        copy("Hello, Linux"),
      ],
    },
    {
      title: "Say Hi",
      icon: "💬",
      actions: [
        action("Say Hi", async () => {
          await toast("Hi!", { style: "success" });
          return close();
        }),
      ],
    },
  ],
});
"##,
    )?;

    // Lets editors resolve the API and its types. At runtime the launcher maps
    // `@backslash/api` to the SDK it ships, so versions always match.
    let sdk = paths::sdk_dir();
    std::fs::write(
        dir.join("deno.json"),
        format!("{{\n  \"imports\": {{\n    \"@backslash/api\": \"{}/mod.ts\"\n  }}\n}}\n", sdk.display()),
    )?;
    std::fs::write(dir.join(".gitignore"), "node_modules\n")?;
    done(&dir, id)
}

const PYTHON_HELLO: &str = r##"from backslash import Action, Copy, Detail, Item, List, close, toast


def say_hi(ctx):
    toast("Hi!", style="success")
    return close()


command = List(
    placeholder="Search greetings…",
    items=[
        Item(
            "Hello, Linux",
            subtitle="Press Enter to see details",
            icon="👋",
            tags=["new"],
            actions=[
                Action("Show Details", lambda ctx: Detail("# Hello!\n\nEdit `commands/hello.py`, then reopen this command.")),
                Copy("Hello, Linux"),
            ],
        ),
        Item("Say Hi", icon="💬", actions=[Action("Say Hi", say_hi)]),
    ],
)
"##;

fn done(dir: &Path, id: &str) -> Result<()> {
    println!("Created {}", dir.display());
    println!();
    println!("  cd {}", dir.display());
    println!("  backslash dev        # open the launcher with your extension loaded");
    println!("  backslash run {id}/hello");
    Ok(())
}

fn title_case(id: &str) -> String {
    id.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
