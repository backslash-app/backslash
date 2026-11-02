import { action, answer, copy } from "@backslash/api";

export default answer((query, { signal }) => {
  if (query === "slow") {
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => resolve(null), 10_000);
      signal.addEventListener("abort", () => {
        clearTimeout(t);
        reject(new Error("aborted"));
      });
    });
  }
  const n = Number(query);
  if (query.trim() === "" || !Number.isFinite(n)) return null;
  const result = String(n * 2);
  return {
    title: result,
    subtitle: `${n} × 2`,
    actions: [copy(result, "Copy Result"), action("Callback", () => {})],
  };
}, { placeholder: "A number to double…" });
