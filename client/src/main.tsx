import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { SkillDetailWindow } from './app/SkillDetailWindow';
// The dock is mounted through its module entry point, not by path: that file is the seam between
import { DockWindow } from './app/dock';
import './styles/tokens.css';
import './styles/base.css';
import './styles/shell.css';
import './styles/chat.css';
import './styles/settings.css';
import './styles/file-kind.css';

declare global {
  interface Window {
    /** Injected by Rust before load when this webview is a skill detail window. */
    __REAL_SKILL__?: string;
    /** Injected by Rust before load when this webview is the right-dock window. */
    __REAL_DOCK__?: boolean;
  }
}

// One bundle serves every window; the flags Rust injects are what tell them apart.
const skillName = window.__REAL_SKILL__;
const isDock = window.__REAL_DOCK__;

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {skillName ? <SkillDetailWindow name={skillName} /> : isDock ? <DockWindow /> : <App />}
  </React.StrictMode>,
);
