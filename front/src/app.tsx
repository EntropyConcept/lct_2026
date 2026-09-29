import { useEffect } from 'preact/hooks';
import { navigation } from './entity/navigation';
import './app.css'
import { Editor } from './features/editor'
import { Header } from './widgets/header'
import { Settings } from './widgets/settings'
import { MapDisplay } from './widgets/map'
import { Panel } from './widgets/bottom-panel'
import { ModeSwitch } from './features/mode-switch'

export function App() {
  useEffect(() => { void navigation.initialize(); }, []);

  return (
    <div>
      <div className='flex flex-col h-[100vh] w-full border-b border-b-gray-200'>
        <Header/>
        <div className='flex flex-1 min-h-0 w-full workspace'>
          <div className='flex min-h-0 sidebar-group'>
            <Settings/>
            <Editor/>
          </div>
          <main className={'min-w-0 flex-1 flex flex-col relative'}>
            <ModeSwitch/>
            <MapDisplay/>
            <Panel/>
          </main>
        </div>
      </div>
    </div>
  )
}
