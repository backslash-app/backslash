import { list } from "@backslash/api";

export default list({
  search: async (query, { signal }) => {
    if (query === "slow") await new Promise((r, reject) => {
      const t = setTimeout(r, 10_000);
      signal.addEventListener("abort", () => { clearTimeout(t); reject(new Error("aborted")); });
    });
    console.log("searching", query);
    return [{ title: `You typed ${query}` }];
  },
});
