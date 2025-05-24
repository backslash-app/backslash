import {
  ChangeEvent,
  Dispatch,
  KeyboardEvent,
  SetStateAction,
  useCallback,
  useEffect,
  useRef,
  useState
} from 'react'
import debounce from 'lodash/debounce'
import { Search } from 'lucide-react'

import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandList,
  CommandShortcut
} from '@renderer/elements/Command'
import { Footer } from '@renderer/elements/Footer'
import { SubCommand } from '@renderer/components/SubCommand'
import { renderElement, winElectron } from '@renderer/lib/utils'
import { useFetchActions, useObserveSelectedOption, useScrollToTop } from '@renderer/hooks'

type CommandPageProps = {
  selectedCommand: CommandT
  setSelectedCommand: Dispatch<SetStateAction<CommandT | null>>
  setCommandSearch: Dispatch<SetStateAction<string>>
}

export const CommandPage = ({
  selectedCommand,
  setSelectedCommand,
  setCommandSearch
}: CommandPageProps) => {
  const [commandParam, setCommandParam] = useState('')
  const [commandResult, setCommandResult] = useState<ResultT[]>([])
  const [isTyping, setIsTyping] = useState(false)
  const [isLoading, setIsLoading] = useState(false)

  const inputRef = useRef(null)
  const commandListRef = useRef<HTMLDivElement | null>(null)

  useScrollToTop(commandListRef, [commandParam])

  const actions = useFetchActions(selectedCommand.plugin.name, selectedCommand.name)
  const selectedValue = useObserveSelectedOption(commandResult)

  const debouncedRunCommand = useCallback(
    debounce(async (command, param) => {
      if (command && winElectron && winElectron.runCommand) {
        setIsLoading(true)
        const result = await winElectron.runCommand(command.plugin.name, command.name, param)
        setCommandResult(result)
        setIsLoading(false)
        setIsTyping(false)
      }
    }, 300),
    []
  )

  useEffect(() => {
    if (selectedCommand) debouncedRunCommand(selectedCommand, commandParam)
  }, [selectedCommand, commandParam, debouncedRunCommand])

  const handleInputChange = (e: ChangeEvent<HTMLInputElement>) => {
    const newValue = e.target.value
    setCommandParam(newValue)
    setIsTyping(true)
  }

  const handleInputKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const isEscapeOrBackspace = e.key === 'Escape'
    const isBackspaceWithEmptyInput = e.key === 'Backspace' && commandParam === ''

    if (isEscapeOrBackspace || isBackspaceWithEmptyInput) {
      e.preventDefault()
      resetCommandSelection()
    }
  }

  const handleActionSelect = async () => {
    if (selectedCommand.plugin.name && selectedCommand.name && selectedValue) {
      await winElectron.runPluginAction(
        selectedCommand.plugin.name,
        selectedCommand.name,
        actions[0].name,
        selectedValue
      )

      winElectron.hideMainWindow()
    }
  }

  const resetCommandSelection = () => {
    setSelectedCommand(null)
    setCommandParam('')
    setCommandResult([])
    setCommandSearch('')
  }

  const renderCommandResult = () => {
    return commandResult?.map((item, index: number) => (
      // WARNING: ID is important here, because it's used in the `useObserveSelectedOption` hook
      // Be careful to send it in data
      <CommandItem key={index} onSelect={handleActionSelect} value={item.data.id}>
        {item.content.map((element, elementIndex) =>
          renderElement(element, `${index}-${elementIndex}`)
        )}
      </CommandItem>
    ))
  }

  const showSubCommand = commandResult && selectedValue && actions.length > 1
  const hasResults = commandResult?.length > 0
  const emptyResult = !commandParam && !isLoading
  const isLoadingResult = isLoading && commandParam
  const noResult = !isTyping && !isLoading && commandParam

  if (selectedCommand.isImmediate) return null

  return (
    <Command loop>
      <div className="flex items-center border-b border-zinc-800 px-3">
        <Search className="mr-2 h-4 w-4 shrink-0 opacity-50" />

        <input
          autoFocus
          className="flex h-11 w-full rounded-md bg-transparent py-3 text-sm outline-hidden placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50"
          value={commandParam}
          onChange={handleInputChange}
          onKeyDown={handleInputKeyDown}
          placeholder="Enter command parameter"
          ref={inputRef}
        />
      </div>

      <CommandList className="h-screen" ref={commandListRef}>
        <CommandEmpty className="flex flex-col h-80 items-center justify-center">
          {isLoadingResult && (
            <div className="flex flex-col items-center">
              <i className="ph ph-hourglass-high text-8xl text-zinc-600 animate-spin" />
              <p className="text-2xl text-zinc-600 italic">Waiting for result...</p>
            </div>
          )}

          {noResult && (
            <div className="flex flex-col items-center">
              <i className="ph ph-empty text-8xl text-zinc-600" />
              <p className="text-2xl text-zinc-600 italic">There is no result.</p>
            </div>
          )}

          {emptyResult && (
            <div className="flex flex-col items-center">
              <i className="ph ph-textbox text-8xl text-zinc-600" />
              <p className="text-2xl text-zinc-600 italic">Type a command parameter.</p>
            </div>
          )}
        </CommandEmpty>

        {hasResults && <CommandGroup heading="Results">{renderCommandResult()}</CommandGroup>}
      </CommandList>

      <Footer>
        <div className="flex flex-row items-center">
          {actions.length > 0 && selectedValue && (
            <span className="flex items-center gap-2 text-xs">
              {actions[0].name}
              <CommandShortcut>↵</CommandShortcut>
            </span>
          )}

          {showSubCommand && (
            <>
              <hr className="ml-4 mr-4 border-0 border-l border-l-zinc-800 h-3" />
              <SubCommand
                actions={actions}
                pluginName={selectedCommand.plugin.name}
                commandName={selectedCommand.name}
                result={selectedValue}
                inputRef={inputRef}
              />
            </>
          )}
        </div>
      </Footer>
    </Command>
  )
}
