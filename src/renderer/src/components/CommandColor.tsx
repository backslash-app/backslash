import { evaluateColorExpression, ResultColorCodes } from '@renderer/lib/color';
import { useMemo } from 'react'

type CommandColorProps = { commandSearch: string }

type MemoResults = {
  isColorCommand: false,
  result: null
} | {
  isColorCommand: true,
  result: ResultColorCodes
}

export const CommandColor = ({ commandSearch }: CommandColorProps) => {
  const { isColorCommand, result } = useMemo<MemoResults>(() => {
    const result = evaluateColorExpression(commandSearch)

    if (result === null) return { isColorCommand: false, result: null }

    return { 
      isColorCommand: true, 
      result
    }
  }, [commandSearch])

  return (
    <>
      {isColorCommand && (
        <div className='flex gap-6 px-4 bg-neutral-900'>
          <div className='py-4 pr-8 shrink-0 space-y-4 text-zinc-300'>
            <p>HEX: #{result.hex}</p>
            <p>RGB: rgb({result.rgb.r}, {result.rgb.g}, {result.rgb.b})</p>
            <p>HSL: hsl({result.hsl.h}, {result.hsl.s}, {result.hsl.l})</p>
          </div>
          <div className='py-4 flex flex-col items-center justify-center w-full gap-4 border-l border-l-neutral-800'>
            <div 
              className='rounded-full w-24 h-24 my-4' 
              style={{
                  backgroundColor: `#${result.hex}`,
                  border: `6px solid rgb(
                    ${Math.max(0, Math.floor(result.rgb.r * 0.8))},
                    ${Math.max(0,Math.floor(result.rgb.g * 0.8))},
                    ${Math.max(0, Math.floor(result.rgb.b * 0.8))})
                  `,
              }}
            />
            <div className='flex flex-col text-start w-full'>
              <span className='text-lg font-medium py-2 border-y border-neutral-800 px-8'>Color: {result.colorDescription}</span>
              <span className='text-sm text-zinc-400 px-8 pt-2'>Lightness: {result.hsl.l}%</span>
              <span className='text-sm text-zinc-400 px-8'>Saturation: {result.hsl.s}%</span>
            </div>
          </div>
        </div>
      )}
    </>
  )
}