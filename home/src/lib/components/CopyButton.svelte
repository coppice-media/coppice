<script lang="ts">
	import CopyIcon from '@lucide/svelte/icons/copy';
	import CheckIcon from '@lucide/svelte/icons/check';
	import { toast } from 'svelte-sonner';
	import { Button } from '@stump/ui/components/ui/button';

	let { value, what }: { value: string; what: string } = $props();

	let copied = $state(false);

	async function copy(): Promise<void> {
		try {
			await navigator.clipboard.writeText(value);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			// Clipboard access needs a secure context; plain-http LAN access falls back to a prompt.
			window.prompt(`Copy the ${what}`, value);
			toast.info(`Clipboard is unavailable over plain http; copy the ${what} from the prompt.`);
		}
	}
</script>

<Button
	variant="ghost"
	size="icon-sm"
	class="shrink-0"
	onclick={copy}
	aria-label={copied ? `${what} copied` : `Copy ${what}`}
	title={`Copy ${what}`}
>
	{#if copied}
		<CheckIcon class="text-emerald-600" />
	{:else}
		<CopyIcon />
	{/if}
</Button>
