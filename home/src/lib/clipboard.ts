import { toast } from 'svelte-sonner'

/**
 * Copies `value` to the clipboard and reports whether it landed there.
 * Clipboard access needs a secure context; over plain-http LAN access the
 * browser refuses, so the value is offered in a prompt the user can copy from
 * by hand instead. `what` names the value in that fallback ("secret", "URL").
 */
export async function copyText(value: string, what: string): Promise<boolean> {
	try {
		await navigator.clipboard.writeText(value)
		return true
	} catch {
		window.prompt(`Copy the ${what}`, value)
		toast.info(`Clipboard is unavailable over plain http; copy the ${what} from the prompt.`)
		return false
	}
}
