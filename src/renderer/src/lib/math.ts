import { evaluate } from 'mathjs'

export function isMathExpression(str: string): boolean {
  try {
    const result = evaluate(str)
    return typeof result === 'number' && isFinite(result)
  } catch {
    return false
  }
}

export function evaluateMathExpression(str: string): string {
  try {
    const result = evaluate(str)
    return typeof result === 'number' ? result.toString() : 'Invalid expression'
  } catch (error) {
    return 'Invalid expression'
  }
}
