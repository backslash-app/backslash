import { exec, run } from "@backslash/api";

// PipeWire first (most current distributions), PulseAudio as a fallback.
export default run<{ level: number }>(async ({ args }) => {
  const level = Math.round(args.level);
  try {
    await exec("wpctl", ["set-volume", "@DEFAULT_AUDIO_SINK@", `${level}%`]);
  } catch (pipewire) {
    try {
      await exec("pactl", ["set-sink-volume", "@DEFAULT_SINK@", `${level}%`]);
    } catch {
      throw new Error(`Could not change the volume (${(pipewire as Error).message})`);
    }
  }
  return `Volume ${level}%`;
});
