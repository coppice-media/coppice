<script lang="ts">
	/**
	 * Highlight colour choice shared by the annotation hub and the reader
	 * panel: no colour, or one of the server-accepted `ANNOTATION_COLORS`.
	 * `value` is `null` for no colour. A colour stored outside the palette (an
	 * older native row) stays shown as the current choice until replaced.
	 */
	import { Button } from '@stump/ui/components/ui/button';
	import { ANNOTATION_COLORS } from '$lib/annotations';

	let { value = $bindable(null) }: { value?: string | null } = $props();

	const custom = $derived(
		value !== null && !(ANNOTATION_COLORS as readonly string[]).includes(value) ? value : null
	);
</script>

<div class="flex flex-wrap items-center gap-1.5" role="group" aria-label="Highlight colour">
	<span class="mr-1 text-sm text-muted-foreground">Colour</span>
	<Button
		type="button"
		size="sm"
		variant={value === null ? 'secondary' : 'ghost'}
		aria-pressed={value === null}
		onclick={() => (value = null)}
	>
		None
	</Button>
	{#if custom}
		<button
			type="button"
			class="size-6 rounded-full ring-2 ring-ring ring-offset-2 ring-offset-background"
			style:background-color={custom}
			aria-pressed="true"
			aria-label={`Current colour ${custom}`}
			title={`Current colour ${custom}`}
		></button>
	{/if}
	{#each ANNOTATION_COLORS as color (color)}
		<button
			type="button"
			class={[
				'size-6 rounded-full ring-1 ring-foreground/20',
				value === color && 'ring-2 ring-ring ring-offset-2 ring-offset-background'
			]}
			style:background-color={color}
			aria-pressed={value === color}
			aria-label={color}
			title={color}
			onclick={() => (value = color)}
		></button>
	{/each}
</div>
