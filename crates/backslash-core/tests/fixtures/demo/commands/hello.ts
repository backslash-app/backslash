import { run } from "@backslash/api";

export default run<{ name: string }>(({ args }) => `Hello ${args.name}`);
