import { Card } from '@renderer/elements/Card'
import { CommandGroup } from '@renderer/elements/Command'
import { evaluateMathExpression, isMathExpression } from '@renderer/lib/math'
import { ArrowRight } from 'lucide-react'
import { useMemo } from 'react'

type CommandMathProps = { commandSearch: string }

export const CommandMath = ({ commandSearch }: CommandMathProps) => {
  const { isMathCommand, resultMath } = useMemo(() => {
    const isMathCommand = isMathExpression(commandSearch)
    if (isMathCommand) {
      return { isMathCommand, resultMath: evaluateMathExpression(commandSearch) }
    }
    return { isMathCommand: false, resultMath: null }
  }, [commandSearch])

  return (
    <>
      {isMathCommand && (
        <div className="flex  items-center justify-center bg-neutral-900 p-1">
          <div className="w-full max-w-2xl space-y-6">
            <h1 className="text-xl font-semibold text-white">Calculator</h1>
            <Card className="rounded-xl border-none bg-neutral-800 p-8 shadow-lg">
              <div className="flex w-full items-center justify-between gap-8 flex-row">
                <div className="flex flex-1 flex-col items-center md:items-start">
                  <span className="text-4xl font-bold text-white md:text-5xl">{commandSearch}</span>
                </div>

                <ArrowRight className="h-8 w-8 text-white md:h-10 md:w-10" />

                <div className="flex flex-1 flex-col items-center md:items-end">
                  <div className="mt-2 rounded-md bg-neutral-700 px-3 py-1 text-sm font-medium text-white">
                    <span className="text-4xl font-bold text-white md:text-5xl">{resultMath}</span>
                  </div>
                </div>
              </div>
            </Card>
          </div>
        </div>
      )}
    </>
  )
}
