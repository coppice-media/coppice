<script lang="ts">
	/**
	 * The newest books in the libraries this user can see, as a compact list:
	 * a thumbnail, the title, its series, and when the scan found it.
	 */
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Cover } from '@stump/ui/components/ui/cover';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import type { DashboardBookCardFragment } from '$lib/graphql/generated/graphql';
	import { clockLabel } from '$lib/dashboard';
	import { absoluteTime, countNoun, relativeTime } from '$lib/format';
	import Widget from './Widget.svelte';

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
	title="Recently added"
	description="The newest books in your libraries."
	href={resolve('/library')}
	{query}
	empty={books.length === 0}
	emptyTitle="No books yet"
	emptyDescription="Scan a library and its books show up here."
	errorTitle="Unable to load recently added books"
	class={className}
>
	{#snippet skeleton()}
		<div class="flex flex-col gap-3">
			{#each { length: 5 } as _, index (index)}
				<div class="flex items-center gap-3">
					<Skeleton class="h-14 w-10 shrink-0 rounded-md" />
					<div class="flex flex-1 flex-col gap-2">
						<Skeleton class="h-4 w-2/3" />
						<Skeleton class="h-3 w-1/3" />
					</div>
				</div>
			{/each}
		</div>
	{/snippet}
	<ul class="-my-1 flex flex-col divide-y">
		{#each books as book (book.id)}
			<li class="flex items-center gap-3 py-2.5">
				<a
					class="block shrink-0"
					href={resolve('/(app)/reader/[mediaId]', { mediaId: book.id })}
					tabindex="-1"
					aria-hidden="true"
				>
					<Cover src={book.thumbnail.url} class="h-14 w-10 rounded-md ring-1 ring-foreground/10">
						{#snippet fallback()}
							<span class="px-1 text-center text-[10px] uppercase">{book.extension || 'Book'}</span>
						{/snippet}
					</Cover>
				</a>
				<div class="flex min-w-0 flex-1 flex-col gap-0.5">
					<a
						class="truncate text-sm font-medium hover:underline"
						href={resolve('/(app)/reader/[mediaId]', { mediaId: book.id })}
					>
						{book.resolvedName}
					</a>
					<span class="truncate text-xs text-muted-foreground">
						<a
							class="hover:underline"
							href={resolve('/(app)/series/[id]', { id: book.series.id })}
						>
							{book.series.resolvedName}
						</a>
						{#if book.audio}
							· {clockLabel(book.audio.durationMs)}
						{:else if book.pages > 0}
							· {countNoun(book.pages, 'page')}
						{/if}
					</span>
				</div>
				<Badge variant="outline" class="hidden uppercase @md/widget:inline-flex">
					{book.extension}
				</Badge>
				<span class="shrink-0 text-xs text-muted-foreground" title={absoluteTime(book.createdAt)}>
					{relativeTime(book.createdAt, now)}
				</span>
			</li>
		{/each}
	</ul>
</Widget>
