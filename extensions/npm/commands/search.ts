import { action, copy, detail, list, open, storage } from "@backslash/api";

interface Package {
  name: string;
  version: string;
  description?: string;
  date: string;
  links: { npm: string; homepage?: string; repository?: string };
  publisher?: { username: string };
}

async function remember(name: string) {
  const recent = (await storage.get<string[]>("recent")) ?? [];
  await storage.set("recent", [name, ...recent.filter((n) => n !== name)].slice(0, 10));
}

function item(pkg: Package) {
  return {
    id: pkg.name,
    title: pkg.name,
    subtitle: pkg.description,
    icon: "package-x-generic",
    tags: [`v${pkg.version}`],
    actions: [
      action("Show Details", async () => {
        await remember(pkg.name);
        return detail({
          title: pkg.name,
          markdown: `# ${pkg.name}\n\n${pkg.description ?? ""}\n\n\`\`\`sh\nnpm install ${pkg.name}\n\`\`\``,
          metadata: {
            Version: pkg.version,
            Published: pkg.date.slice(0, 10),
            Publisher: pkg.publisher?.username ?? "—",
          },
          actions: [open(pkg.links.npm, "Open on npmjs.com"), copy(`npm install ${pkg.name}`, "Copy Install Command")],
        });
      }),
      open(pkg.links.npm, "Open on npmjs.com"),
      ...(pkg.links.repository ? [open(pkg.links.repository, "Open Repository", { icon: "folder-remote" })] : []),
      copy(`npm install ${pkg.name}`, "Copy npm install", { shortcut: "<Ctrl><Shift>c" }),
      copy(pkg.name, "Copy Package Name"),
    ],
  };
}

export default list({
  placeholder: "Search npm packages…",
  empty: { title: "Type to search the npm registry", icon: "system-search" },
  async search(query, { signal }) {
    if (!query.trim()) {
      const recent = (await storage.get<string[]>("recent")) ?? [];
      return recent.map((name) => ({
        title: name,
        section: "Recently viewed",
        icon: "document-open-recent",
        actions: [open(`https://www.npmjs.com/package/${name}`, "Open on npmjs.com")],
      }));
    }
    const url = `https://registry.npmjs.org/-/v1/search?size=20&text=${encodeURIComponent(query)}`;
    const res = await fetch(url, { signal });
    if (!res.ok) throw new Error(`npm registry answered ${res.status}`);
    const { objects } = await res.json() as { objects: { package: Package }[] };
    return objects.map((o) => item(o.package));
  },
});
