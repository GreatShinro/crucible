import { EventListenerDashboard } from './components/EventListenerDashboard';
import './App.css';

function App() {
  return (
    <div className="app-container">
      <header className="app-header">
        <h1>Crucible Dashboard</h1>
        <div className="header-badge">Mainnet Beta</div>
      </header>
      
      <main className="app-main">
        <EventListenerDashboard />
      </main>
    </div>
  );
}

export default App;
