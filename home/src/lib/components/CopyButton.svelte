<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import { Button, type ButtonSize } from '@stump/ui/components/ui/button';
	import { cn } from '@stump/ui/utils.js';
	import { copyText } from '$lib/clipboard';

	let {
		value,
		what,
		size = 'icon-sm',
		class: className
	}: { value: string; what: string; size?: ButtonSize; class?: string } = $props();

	let copied = $state(false);

	async function copy(): Promise<void> {
		if (!(await copyText(value, what))) return;
		copied = true;
		setTimeout(() => (copied = false), 1500);
	}
</script>

<Button
	variant="ghost"
	{size}
	class={cn('shrink-0', className)}
	onclick={copy}
	aria-label={copied ? `${what} copied` : `Copy ${what}`}
	title={`Copy ${what}`}
>
	{#if copied}
		<CheckIcon class="text-primary" />
	{:else}
		<CopyIcon />
	{/if}
</Button>
