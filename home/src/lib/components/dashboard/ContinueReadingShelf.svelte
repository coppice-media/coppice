<script lang="ts">
	import { resolve } from '$app/paths';
	import { Button } from '@stump/ui/components/ui/button';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import type { DashboardBookCardFragment } from '$lib/graphql/generated/graphql';
	import ContinueReadingCard from './ContinueReadingCard.svelte';
	import Widget from './Widget.svelte';

	// `keepReading` is the unfinished heads ordered by `updated_at`, so the
	// shelf reads left to right as "most recently touched, on any device".
	// A narrow widget (a phone) has no room for a rail, so it shows a
	// three-column grid of the first `NARROW_LIMIT` instead of scrolling sideways.
	const NARROW_LIMIT = 6;

	let {
		books,
		query,
		now = new Date(),
		class: className
	}: {
		books: readonly DashboardBookCardFragment[];
		query: { isPending: boolean; error: unknown; refetch: () => unknown };
		now?: Date;
		class?: string;
	} = $props();
</script>

<Widget
	title="Continue reading"
	description="Where you left off, on every device."
	href={resolve('/library')}
	hrefLabel="Library"
	{query}
	empty={books.length === 0}
	emptyTitle="Nothing in progress"
	emptyDescription="Open a book here or on any synced device and it shows up on this shelf."
	errorTitle="Unable to load your open books"
	class={className}
>
	{#snippet skeleton()}
		<div class="grid grid-cols-3 gap-3 @md/widget:flex @md/widget:gap-5 @md/widget:overflow-hidden">
			{#each { length: 6 } as _, index (index)}
				<div class={['flex-col gap-3 @md/widget:w-36 @md/widget:shrink-0', index < 3 ? 'flex' : 'hidden @md/widget:flex']}>
					<Skeleton class="aspect-2/3 w-full rounded-lg" />
					<Skeleton class="h-4 w-5/6" />
					<Skeleton class="h-3 w-1/2" />
				</div>
			{/each}
		</div>
	{/snippet}
	{#snippet emptyActions()}
		<Button size="sm" variant="outline" href={resolve('/library')}>Browse the library</Button>
	{/snippet}
	<ul
		class="grid grid-cols-3 gap-3 @md/widget:-mx-(--card-spacing) @md/widget:flex @md/widget:snap-x @md/widget:gap-5 @md/widget:overflow-x-auto @md/widget:px-(--card-spacing) @md/widget:pb-2"
		aria-label="Books in progress"
	>
		{#each books as book, index (book.id)}
			<li class={['h-full', index < NARROW_LIMIT ? 'flex' : 'hidden @md/widget:flex']}>
				<ContinueReadingCard {book} {now} />
			</li>
		{/each}
		{#if books.length > NARROW_LIMIT}
			<li class="col-span-3 @md/widget:hidden">
				<Button variant="outline" size="sm" class="w-full" href={resolve('/library')}>
					See all {books.length}
				</Button>
			</li>
		{/if}
	</ul>
</Widget>
