import { normalizeHex, hexToRgb } from './colorHelper/hex'
import { parseHsl, hslToRgb } from './colorHelper/hsl'
import { parseRgb, rgbToHex, rgbToHsl } from './colorHelper/rgb'

export type ResultColorCodes = {
  hex: string
  rgb: { r: number; g: number; b: number }
  hsl: { h: number; s: number; l: number }
  colorDescription: string
}

export function evaluateColorExpression(str: string): ResultColorCodes | null {
  // Check for HEX
  if (str.startsWith('#') && str.length > 3) {
    const hex = normalizeHex(str)

    if (hex === null) return null

    const rgb = hexToRgb(hex)
    const hsl = rgbToHsl(rgb)

    return {
      hex,
      rgb,
      hsl,
      colorDescription: getColorInfo(hex, hsl)
    }
  }

  // Check for RGB
  if (str.startsWith('rgb(')) {
    const rgb = parseRgb(str)

    if (rgb === null) return null

    const hex = rgbToHex(rgb)
    const hsl = rgbToHsl(rgb)

    return {
      hex,
      rgb,
      hsl,
      colorDescription: getColorInfo(hex, hsl)
    }
  }

  // Check for HSL
  // TODO: check for other hsl formats too
  if (str.startsWith('hsl(')) {
    const hsl = parseHsl(str)

    if (hsl === null) return null

    const rgb = hslToRgb(hsl)
    const hex = rgbToHex(rgb)

    if (normalizeHex !== null) {
      return {
        hex,
        rgb,
        hsl,
        colorDescription: getColorInfo(hex, hsl)
      }
    }
  }

  return null
}

// Helper: Get color name/description
export function getColorInfo(hex: string, hsl: { h: number; s: number; l: number }) {
  const { h, s, l } = hsl

  // Exact match for white and black
  if (hex.toLowerCase() === 'ffffff') return 'White'
  if (hex.toLowerCase() === '000000') return 'Black'

  let description = ''

  // Lightness description
  if (l < 20) description += 'Very Dark '
  else if (l < 40) description += 'Dark '
  else if (l > 80) description += 'Light '
  else if (l > 60) description += 'Bright '

  // Saturation description
  if (s < 10) description += 'Gray'
  else if (s < 30) description += 'Muted '

  // Hue description
  if (s >= 10) {
    if (h >= 0 && h < 15) description += 'Red'
    else if (h >= 15 && h < 45) description += 'Orange'
    else if (h >= 45 && h < 75) description += 'Yellow'
    else if (h >= 75 && h < 150) description += 'Green'
    else if (h >= 150 && h < 210) description += 'Cyan'
    else if (h >= 210 && h < 270) description += 'Blue'
    else if (h >= 270 && h < 330) description += 'Purple'
    else description += 'Red'
  }

  return description.trim() || 'Neutral'
}
