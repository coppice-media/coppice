<script lang="ts">
	import { resolve } from '$app/paths';
	import type { DashboardBookCardFragment } from '$lib/graphql/generated/graphql';
	import { minutesLabel, relativeTime } from '$lib/format';

	let {
		book,
		showProgress = false,
		now = new Date()
	}: {
		book: DashboardBookCardFragment;
		showProgress?: boolean;
		now?: Date;
	} = $props();

	let coverFailed = $state(false);

	// `percentageCompleted` is the readthrough's whole-publication progression
	// (0–1) and is the only progress an EPUB head carries; paged books also
	// report a page, which is the better label. The scalar arrives as a
	// decimal string or a number depending on the transport, hence `Number`.
	const ratio = $derived.by(() => {
		const raw = book.readProgress?.percentageCompleted;
		const value = raw === null || raw === undefined ? Number.NaN : Number(raw);
		if (Number.isFinite(value) && value > 0) return Math.min(1, value);
		const page = book.readProgress?.page;
		if (page && book.pages > 0) return Math.min(1, page / book.pages);
		return null;
	});
	const progressLabel = $derived.by(() => {
		const page = book.readProgress?.page;
		if (page && book.pages > 0) return `Page ${page} of ${book.pages}`;
		if (ratio !== null) return `${Math.round(ratio * 100)}% read`;
		return 'Not started';
	});
	const elapsed = $derived(book.readProgress?.elapsedSeconds ?? 0);
</script>

<a
	class="group flex w-36 shrink-0 flex-col gap-2 rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring"
	href={resolve('/(app)/reader/[mediaId]', { mediaId: book.id })}
>
	<div
		class="relative aspect-2/3 w-full overflow-hidden rounded-md border bg-muted shadow-sm transition-shadow group-hover:shadow-md"
	>
		{#if coverFailed}
			<span
				class="absolute inset-0 flex items-center justify-center px-2 text-center text-xs text-muted-foreground uppercase"
			>
				{book.extension}
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
		{#if showProgress && ratio !== null}
			<div class="absolute inset-x-0 bottom-0 h-1 bg-background/60">
				<div class="h-full bg-primary" style="width: {Math.round(ratio * 100)}%"></div>
			</div>
		{/if}
	</div>
	<div class="flex flex-col gap-0.5">
		<span class="line-clamp-2 text-sm leading-tight font-medium group-hover:underline">
			{book.resolvedName}
		</span>
		<span class="line-clamp-1 text-xs text-muted-foreground">{book.series.resolvedName}</span>
		{#if showProgress}
			<span class="text-xs tabular-nums text-muted-foreground">
				{progressLabel}{elapsed >= 60 ? ` · ${minutesLabel(Math.round(elapsed / 60))}` : ''}
			</span>
		{:else}
			<span class="text-xs text-muted-foreground">Added {relativeTime(book.createdAt, now)}</span>
		{/if}
	</div>
</a>
