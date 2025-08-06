// Helper: Validate and parse HSL input
export function parseHsl(input: string) {
  if (typeof input !== 'string') return null

  // Remove whitespace and convert to lowercase
  const cleaned = input.trim().toLowerCase()

  // Match various HSL formats
  const hslMatch = cleaned.match(/^hsl\s*\(\s*(\d+)\s*,\s*(\d+)%?\s*,\s*(\d+)%?\s*\)$/)
  const spaceSeparated = cleaned.match(/^(\d+)\s+(\d+)%?\s+(\d+)%?$/)
  const commaSeparated = cleaned.match(/^(\d+)\s*,\s*(\d+)%?\s*,\s*(\d+)%?$/)

  let h: number, s: number, l: number

  if (hslMatch) {
    ;[, h, s, l] = hslMatch.map(Number)
  } else if (spaceSeparated) {
    ;[, h, s, l] = spaceSeparated.map(Number)
  } else if (commaSeparated) {
    ;[, h, s, l] = commaSeparated.map(Number)
  } else {
    return null
  }

  // Validate HSL values
  if (h < 0 || h > 360 || s < 0 || s > 100 || l < 0 || l > 100) {
    return null
  }

  return { h, s, l }
}

// Helper: Convert HSL to RGB
export function hslToRgb({ h, s, l }) {
  h /= 360
  s /= 100
  l /= 100

  const hue2rgb = (p: number, q: number, t: number) => {
    if (t < 0) t += 1
    if (t > 1) t -= 1
    if (t < 1 / 6) return p + (q - p) * 6 * t
    if (t < 1 / 2) return q
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6
    return p
  }

  let r: number, g: number, b: number

  if (s === 0) {
    r = g = b = l // achromatic
  } else {
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s
    const p = 2 * l - q
    r = hue2rgb(p, q, h + 1 / 3)
    g = hue2rgb(p, q, h)
    b = hue2rgb(p, q, h - 1 / 3)
  }

  return {
    r: Math.round(r * 255),
    g: Math.round(g * 255),
    b: Math.round(b * 255)
  }
}
