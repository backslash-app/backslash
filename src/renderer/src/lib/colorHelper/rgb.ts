// Helper: Validate and parse RGB input
export function parseRgb(input: string) {
  if (typeof input !== 'string') return null

  // Remove whitespace and convert to lowercase
  const cleaned = input.trim().toLowerCase()

  // Match various RGB formats
  const rgbMatch = cleaned.match(/^rgb\s*\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)$/)
  const spaceSeparated = cleaned.match(/^(\d+)\s+(\d+)\s+(\d+)$/)
  const commaSeparated = cleaned.match(/^(\d+)\s*,\s*(\d+)\s*,\s*(\d+)$/)

  let r: number, g: number, b: number

  if (rgbMatch) {
    ;[, r, g, b] = rgbMatch.map(Number)
  } else if (spaceSeparated) {
    ;[, r, g, b] = spaceSeparated.map(Number)
  } else if (commaSeparated) {
    ;[, r, g, b] = commaSeparated.map(Number)
  } else {
    return null
  }

  // Validate RGB values are in range 0-255
  if (r < 0 || r > 255 || g < 0 || g > 255 || b < 0 || b > 255) {
    return null
  }

  return { r, g, b }
}

// Helper: Convert RGB to hex
export function rgbToHex({ r, g, b }) {
  const toHex = (n: number) => {
    const hex = Math.round(n).toString(16)
    return hex.length === 1 ? '0' + hex : hex
  }
  return toHex(r) + toHex(g) + toHex(b)
}

// Helper: Convert RGB to HSL
export function rgbToHsl({ r, g, b }) {
  r /= 255
  g /= 255
  b /= 255
  const max = Math.max(r, g, b),
    min = Math.min(r, g, b)
  let h = 0
  let s = 0
  let l = (max + min) / 2

  if (max === min) {
    h = s = 0 // achromatic
  } else {
    const d = max - min
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
    switch (max) {
      case r:
        h = (g - b) / d + (g < b ? 6 : 0)
        break
      case g:
        h = (b - r) / d + 2
        break
      case b:
        h = (r - g) / d + 4
        break
    }
    h = h / 6
  }

  return {
    h: Math.round(h * 360),
    s: Math.round(s * 100),
    l: Math.round(l * 100)
  }
}
