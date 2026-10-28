import { action, close, menuBar } from "@backslash/api";

export default menuBar({
  title: "2",
  icon: "bs-bell",
  items: (ctx) => [
    { id: "launch", title: ctx.launchType, enabled: false },
    { title: "Inbox", section: "Mail", items: [{ title: "Archive", action: action("Archive", () => close("Archived")) }] },
  ],
});
