import React from 'react'
import { createRoot } from 'react-dom/client'
import { registerSW } from 'virtual:pwa-register'

import App from './App'

function registerServiceWorkerWhenIdle() {
	if (!import.meta.env.PROD || !('serviceWorker' in navigator)) return

	const scheduleRegistration = () => {
		if ('requestIdleCallback' in globalThis) {
			globalThis.requestIdleCallback(registerSW)
		} else {
			globalThis.setTimeout(registerSW, 0)
		}
	}

	if (document.readyState === 'complete') {
		scheduleRegistration()
		return
	}

	globalThis.addEventListener('load', scheduleRegistration, { once: true })
}

const rootElement = document.getElementById('root')

if (!rootElement) {
	throw new Error('Root element not found')
}

const root = createRoot(rootElement)
root.render(
	<React.StrictMode>
		<App />
	</React.StrictMode>,
)

registerServiceWorkerWhenIdle()
