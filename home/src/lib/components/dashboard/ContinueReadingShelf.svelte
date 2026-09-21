<script lang="ts">
	import { resolve } from '$app/paths';
	import { Button } from '@stump/ui/components/ui/button';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import type { DashboardBookCardFragment } from '$lib/graphql/generated/graphql';
	import ContinueReadingCard from './ContinueReadingCard.svelte';
	import Widget from './Widget.svelte';

	// `keepReading` is the unfinished heads ordered by `updated_at`, so the
	// shelf reads left to right as "most recently touched, on any device".
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
		<div class="flex gap-5 overflow-hidden">
			{#each { length: 6 } as _, index (index)}
				<div class="flex w-36 shrink-0 flex-col gap-3">
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
		class="-mx-(--card-spacing) flex snap-x gap-5 overflow-x-auto px-(--card-spacing) pb-2"
		aria-label="Books in progress"
	>
		{#each books as book (book.id)}
			<li class="flex h-full">
				<ContinueReadingCard {book} {now} />
			</li>
		{/each}
	</ul>
</Widget>
