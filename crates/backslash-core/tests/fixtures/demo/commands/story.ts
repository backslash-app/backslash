import { detail } from "@backslash/api";

export default detail({
  async *markdown() {
    let text = "";
    for (const word of ["Once", "upon", "a", "time"]) {
      await new Promise((r) => setTimeout(r, 100));
      yield (text += (text ? " " : "") + word);
    }
  },
});
