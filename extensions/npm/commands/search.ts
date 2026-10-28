import { action, copy, detail, Icons, type Item, list, meta, open, storage } from "@backslash/api";

interface Package {
  name: string;
  version: string;
  description?: string;
  keywords?: string[];
  date: string;
  license?: string;
  links: { npm: string; homepage?: string; repository?: string };
  publisher?: { username: string };
}

interface Result {
  package: Package;
  downloads?: { weekly?: number };
  dependents?: number | string;
}

async function remember(name: string) {
  const recent = (await storage.get<string[]>("recent")) ?? [];
  await storage.set("recent", [name, ...recent.filter((n) => n !== name)].slice(0, 10));
}

/** 1234567 → "1.2M" */
export function compact(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1).replace(/\.0$/, "")}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1).replace(/\.0$/, "")}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1).replace(/\.0$/, "")}k`;
  return String(n);
}

/** "git+https://github.com/facebook/react.git" → "github.com/facebook/react" */
export const shortUrl = (url: string) =>
  url.replace(/^git\+/, "").replace(/^\w+:\/\//, "").replace(/\.git$/, "").replace(/\/$/, "").split("/").slice(0, 3)
    .join("/");

function metadata(pkg: Package) {
  return [
    meta.text("Version", pkg.version, { icon: Icons.tag }),
    meta.text("Published", pkg.date.slice(0, 10), { icon: Icons.calendar }),
    ...(pkg.license ? [meta.text("License", pkg.license, { icon: Icons.shield })] : []),
    ...(pkg.publisher ? [meta.person("Publisher", pkg.publisher.username)] : []),
    ...(pkg.keywords?.length ? [meta.tags("Keywords", pkg.keywords.slice(0, 6))] : []),
    meta.separator(),
    meta.link("npm", `npmjs.com/package/${pkg.name}`, pkg.links.npm),
    ...(pkg.links.repository ? [meta.link("Repository", shortUrl(pkg.links.repository), pkg.links.repository)] : []),
    ...(pkg.links.homepage ? [meta.link("Homepage", shortUrl(pkg.links.homepage), pkg.links.homepage)] : []),
  ];
}

function item({ package: pkg, downloads, dependents }: Result): Item {
  const stats = [
    ...(downloads?.weekly !== undefined ? [{ label: "Weekly downloads", value: compact(downloads.weekly) }] : []),
    ...(dependents !== undefined ? [{ label: "Dependents", value: compact(Number(dependents)) }] : []),
  ];
  return {
    id: pkg.name,
    title: pkg.name,
    subtitle: pkg.description,
    icon: { source: Icons.package, tint: "red" },
    accessories: [
      ...(downloads?.weekly !== undefined
        ? [{ icon: Icons.download, text: compact(downloads.weekly), tooltip: "Weekly downloads" }]
        : []),
      { tag: `v${pkg.version}` },
    ],
    detail: { stats, metadata: metadata(pkg) },
    actions: [
      action("Show Details", async () => {
        await remember(pkg.name);
        return detail({
          title: pkg.name,
          markdown: `# ${pkg.name}\n\n${pkg.description ?? ""}\n\n\`\`\`sh\nnpm install ${pkg.name}\n\`\`\``,
          stats,
          metadata: metadata(pkg),
          actions: [open(pkg.links.npm, "Open on npmjs.com"), copy(`npm install ${pkg.name}`, "Copy Install Command")],
        });
      }, { icon: Icons.info }),
      open(pkg.links.npm, "Open on npmjs.com"),
      ...(pkg.links.repository ? [open(pkg.links.repository, "Open Repository", { icon: Icons.repository })] : []),
      copy(`npm install ${pkg.name}`, "Copy npm install", { shortcut: "<Ctrl><Shift>c", section: "Copy" }),
      copy(pkg.name, "Copy Package Name", { section: "Copy" }),
    ],
  };
}

export default list({
  placeholder: "Search npm packages…",
  empty: { title: "Type to search the npm registry", icon: Icons.search },
  async search(query, { signal }) {
    if (!query.trim()) {
      const recent = (await storage.get<string[]>("recent")) ?? [];
      return recent.map((name) => ({
        title: name,
        section: "Recently viewed",
        icon: Icons.history,
        actions: [open(`https://www.npmjs.com/package/${name}`, "Open on npmjs.com")],
      }));
    }
    const url = `https://registry.npmjs.org/-/v1/search?size=20&text=${encodeURIComponent(query)}`;
    const res = await fetch(url, { signal });
    if (!res.ok) throw new Error(`npm registry answered ${res.status}`);
    const { objects } = await res.json() as { objects: Result[] };
    return objects.map(item);
  },
});
