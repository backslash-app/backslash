import { action, close, exec, list } from "@backslash/api";

const power = (title: string, icon: string, program: string, args: string[], confirm?: string) => ({
  title,
  icon,
  actions: [
    action(title, async () => {
      await exec(program, args);
      return close();
    }, { confirm, destructive: Boolean(confirm) }),
  ],
});

export default list({
  placeholder: "Power…",
  items: [
    power("Lock Screen", "system-lock-screen", "loginctl", ["lock-session"]),
    power("Suspend", "weather-clear-night", "systemctl", ["suspend"]),
    power("Restart", "system-reboot", "systemctl", ["reboot"], "Restart now?"),
    power("Shut Down", "system-shutdown", "systemctl", ["poweroff"], "Shut down now?"),
    power("Log Out", "system-log-out", "loginctl", ["terminate-user", "self"], "Log out now?"),
  ],
});
