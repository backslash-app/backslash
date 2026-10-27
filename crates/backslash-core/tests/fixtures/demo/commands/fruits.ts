import { action, detail, list, refresh, storage } from "@backslash/api";

export default list({
  items: async () => {
    const count = (await storage.get<number>("count")) ?? 0;
    return [
      { title: "Apple", tags: [`clicked ${count}`], actions: [action("Count", async () => {
        await storage.set("count", count + 1);
        return refresh();
      })] },
      { title: "Banana", actions: [action("Details", () => detail("# Banana"))] },
    ];
  },
});
