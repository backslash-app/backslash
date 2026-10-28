import { action, close, type Color, exec, Icons, list } from "@backslash/api";

const power = (title: string, icon: string, tint: Color, program: string, args: string[], confirm?: string) => ({
  title,
  icon: { source: icon, tint },
  actions: [
    action(title, async () => {
      await exec(program, args);
      return close();
    }, { icon, confirm, destructive: Boolean(confirm) }),
  ],
});

export default list({
  placeholder: "Power…",
  items: [
    power("Lock Screen", Icons.lock, "blue", "loginctl", ["lock-session"]),
    power("Suspend", Icons.moon, "purple", "systemctl", ["suspend"]),
    power("Restart", Icons.refresh, "orange", "systemctl", ["reboot"], "Restart now?"),
    power("Shut Down", Icons.power, "red", "systemctl", ["poweroff"], "Shut down now?"),
    power("Log Out", Icons.arrowRight, "gray", "loginctl", ["terminate-user", "self"], "Log out now?"),
  ],
});
