// Helper: Validate and normalize hex color
export function normalizeHex(hex: string) {
  hex = hex.trim().replace(/^#/, '')
  if (/^[0-9a-fA-F]{3}$/.test(hex)) {
    // Expand shorthand (e.g. "abc" -> "aabbcc")
    hex = hex
      .split('')
      .map((x: string) => x + x)
      .join('')
  }
  if (/^[0-9a-fA-F]{6}$/.test(hex)) {
    return hex.toLowerCase()
  }
  return null
}

// Helper: Convert hex to RGB
export function hexToRgb(hex: string) {
  const n = parseInt(hex, 16)
  return {
    r: (n >> 16) & 255,
    g: (n >> 8) & 255,
    b: n & 255
  }
}
