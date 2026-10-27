import { action, copy, exec, list, refresh } from "@backslash/api";

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

export default list({
  placeholder: "Filter processes…",
  items: async () =>
    (await processes()).slice(0, 200).map((p) => ({
      id: p.pid,
      title: p.name,
      subtitle: p.command,
      icon: "application-x-executable",
      keywords: [p.pid, p.user],
      accessories: [
        { text: `${p.cpu.toFixed(1)}% CPU`, color: p.cpu > 50 ? "red" : undefined },
        { text: `${p.memMb.toFixed(0)} MB` },
        { tag: p.pid },
      ],
      actions: [
        action("Stop Process", async () => {
          await exec("kill", [p.pid]);
          return refresh();
        }, { icon: "process-stop", destructive: true }),
        action("Force Kill", async () => {
          await exec("kill", ["-9", p.pid]);
          return refresh();
        }, { icon: "process-stop", destructive: true, confirm: `Force kill ${p.name}?`, shortcut: "<Ctrl><Shift>k" }),
        copy(p.pid, "Copy PID", { shortcut: "<Ctrl><Shift>c" }),
      ],
    })),
});
