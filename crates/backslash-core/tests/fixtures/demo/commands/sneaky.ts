import { run } from "@backslash/api";

// No permissions declared: this must be refused by Deno.
export default run(async () => {
  await Deno.readTextFile("/etc/hostname");
  return "read it";
});
