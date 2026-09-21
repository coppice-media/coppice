<script lang="ts">
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import type { SocialShareOverlay } from '$lib/social';
	import { LISEUR_COLORS } from '$lib/social';

	interface Props {
		overlay: SocialShareOverlay;
		busy?: boolean;
		onToggleVisibility?: (overlay: SocialShareOverlay) => void;
	}

	let { overlay, busy = false, onToggleVisibility }: Props = $props();

	const kindLabel = $derived(overlay.kind.replaceAll('_', ' '));
	const color = $derived(overlay.color ? LISEUR_COLORS[overlay.color] : null);
</script>

<Card class={overlay.hidden ? 'border-dashed opacity-70' : ''}>
	<CardHeader class="flex-row items-start justify-between gap-3 space-y-0">
		<div class="min-w-0">
			<div class="flex flex-wrap items-center gap-2">
				<CardTitle class="text-base">{kindLabel}</CardTitle>
				{#if overlay.hidden}
					<Badge variant="secondary">Hidden for you</Badge>
				{/if}
			</div>
			<p class="mt-1 text-xs text-muted-foreground">Shared annotation</p>
		</div>
		{#if color}
			<span
				class="size-5 shrink-0 rounded-full border border-foreground/30"
				style:background-color={color.hex}
				role="img"
				aria-label="Highlight color: {color.label}"
			></span>
		{/if}
	</CardHeader>
	<CardContent class="flex flex-col gap-3 text-sm">
		{#if overlay.excerpt}
			<blockquote class="border-l-2 pl-3 text-muted-foreground">{overlay.excerpt}</blockquote>
		{/if}
		{#if overlay.body}
			<p class="whitespace-pre-wrap">{overlay.body}</p>
		{/if}
		{#if overlay.percentage !== null && overlay.percentage !== undefined}
			<p class="text-xs text-muted-foreground">Shared progress: {Math.round(overlay.percentage)}%</p>
		{/if}
		<p class="text-xs text-muted-foreground">
			This is a read-only snapshot. It does not expose the reader's locator, device, or session timing.
		</p>
	</CardContent>
	<CardFooter class="border-t pt-4">
		<Button
			type="button"
			size="sm"
			variant="outline"
			disabled={busy}
			aria-pressed={overlay.hidden}
			onclick={() => onToggleVisibility?.(overlay)}
		>
			{#if overlay.hidden}
				<EyeIcon class="mr-1 size-4" />
				Show overlay
			{:else}
				<EyeOffIcon class="mr-1 size-4" />
				Hide overlay
			{/if}
		</Button>
	</CardFooter>
</Card>
