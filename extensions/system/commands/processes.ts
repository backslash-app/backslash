import { action, copy, exec, Icons, list, meta, refresh } from "@backslash/api";

interface Process {
  pid: string;
  user: string;
  cpu: number;
  memMb: number;
  name: string;
  command: string;
}

async function processes(): Promise<Process[]> {
  const out = await exec("ps", ["-eo", "pid=,user=,%cpu=,rss=,comm=,args=", "--sort=-%cpu"]);
  return out.split("\n").map((line) => {
    const [pid, user, cpu, rss, name, ...args] = line.trim().split(/\s+/);
    return { pid, user, cpu: Number(cpu), memMb: Number(rss) / 1024, name, command: args.join(" ") };
  });
}

let total = 0;

export default list({
  placeholder: "Filter processes…",
  status: (items) => `${items.length} of ${total} processes · by CPU`,
  items: async () => {
    const all = await processes();
    total = all.length;
    return all.slice(0, 200).map((p) => ({
      id: p.pid,
      title: p.name,
      subtitle: p.command,
      icon: { source: Icons.cpu, tint: p.cpu > 50 ? "red" : p.cpu > 10 ? "orange" : "gray" },
      keywords: [p.pid, p.user],
      accessories: [
        { text: `${p.cpu.toFixed(1)}%`, color: p.cpu > 50 ? "red" : undefined, tooltip: "CPU" },
        { text: `${p.memMb.toFixed(0)} MB`, tooltip: "Memory" },
      ],
      detail: {
        stats: [{ label: "CPU", value: `${p.cpu.toFixed(1)}%` }, {
          label: "Memory",
          value: `${p.memMb.toFixed(0)} MB`,
        }],
        metadata: [
          meta.text("PID", p.pid, { icon: Icons.hash }),
          meta.person("User", p.user),
          meta.progress("CPU", Math.min(p.cpu / 100, 1), `${p.cpu.toFixed(1)}%`),
          meta.separator(),
          meta.text("Command", p.command),
        ],
      },
      actions: [
        action("Stop Process", async () => {
          await exec("kill", [p.pid]);
          return refresh();
        }, { icon: Icons.stop, destructive: true }),
        action("Force Kill", async () => {
          await exec("kill", ["-9", p.pid]);
          return refresh();
        }, { icon: Icons.cross, destructive: true, confirm: `Force kill ${p.name}?`, shortcut: "<Ctrl><Shift>k" }),
        copy(p.pid, "Copy PID", { shortcut: "<Ctrl><Shift>c", section: "Copy" }),
        copy(p.command, "Copy Command", { section: "Copy" }),
      ],
    }));
  },
});
