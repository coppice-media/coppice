<script lang="ts">
	/**
	 * One book on the *Continue reading* shelf: cover, title, series, and how
	 * far its head is. The cover is a mouse target only — *Open* is the single
	 * focusable link, so a shelf of twelve books is twelve tab stops, not
	 * twenty-four.
	 */
	import { resolve } from '$app/paths';
	import { Button } from '@stump/ui/components/ui/button';
	import { Progress } from '@stump/ui/components/ui/progress';
	import { cn } from '@stump/ui/utils.js';
	import type { DashboardBookCardFragment } from '$lib/graphql/generated/graphql';
	import { headProgress, durationLabel } from '$lib/dashboard';
	import { absoluteTime, relativeTime } from '$lib/format';

	let {
		book,
		now = new Date(),
		class: className
	}: {
		book: DashboardBookCardFragment;
		now?: Date;
		class?: string;
	} = $props();

	let coverFailed = $state(false);

	const href = $derived(resolve('/(app)/reader/[mediaId]', { mediaId: book.id }));
	const progress = $derived(headProgress(book));
	const percent = $derived(progress.ratio === null ? null : Math.round(progress.ratio * 100));
	const elapsedSeconds = $derived(book.readProgress?.elapsedSeconds ?? 0);
	const lastRead = $derived(book.readProgress?.updatedAt ?? null);
	const coverMissing = $derived(!book.thumbnail.url || coverFailed);
	const elapsedLabel = $derived(elapsedSeconds > 0 ? durationLabel(elapsedSeconds * 1000) : null);
</script>

<article class={cn('flex h-full min-w-0 flex-col gap-3 @md/widget:w-36 @md/widget:shrink-0 @md/widget:snap-start', className)}>
	<a
		class="relative block aspect-2/3 w-full overflow-hidden rounded-lg bg-muted ring-1 ring-foreground/10 transition-shadow hover:shadow-md"
		{href}
		tabindex="-1"
		aria-hidden="true"
	>
		{#if coverMissing}
			<span
				class="absolute inset-0 flex items-center justify-center px-2 text-center text-xs text-muted-foreground uppercase"
			>
				{book.extension || 'Book'}
			</span>
		{:else}
			<img
				class="size-full object-cover"
				src={book.thumbnail.url}
				alt=""
				loading="lazy"
				onerror={() => (coverFailed = true)}
			/>
		{/if}
	</a>
	<div class="flex min-w-0 flex-1 flex-col gap-1.5 @md/widget:min-h-24">
		<span class="line-clamp-2 text-sm leading-snug font-medium" title={book.resolvedName}>
			{book.resolvedName}
		</span>
		<span class="line-clamp-1 text-xs text-muted-foreground @max-md/widget:hidden" title={book.series.resolvedName}>
			{book.series.resolvedName}
		</span>
		<Progress
			value={percent ?? 0}
			class="mt-1 h-1"
			aria-label={`${book.resolvedName}: ${progress.label}`}
		/>
		<span class="text-xs tabular-nums text-muted-foreground @max-md/widget:truncate">
			{#if percent !== null}{percent}% ·{' '}{/if}{progress.label}{#if elapsedLabel}{' '}· {elapsedLabel}{/if}
		</span>
		{#if lastRead}
			<span class="text-xs text-muted-foreground @max-md/widget:hidden" title={absoluteTime(lastRead)}>
				Last read {relativeTime(lastRead, now)}
			</span>
		{/if}
	</div>
	<Button class="mt-auto" size="sm" variant="outline" {href} aria-label={`Open ${book.resolvedName}`}>Open</Button>
</article>
