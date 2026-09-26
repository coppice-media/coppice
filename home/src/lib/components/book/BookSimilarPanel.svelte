<script lang="ts">
	import ArrowUpRightIcon from '@lucide/svelte/icons/arrow-up-right';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import type { SimilarBook } from '$lib/book/detail';
	import { labelKind, matchTier } from '$lib/book/detail';

	let { books = [] }: { books?: SimilarBook[] } = $props();
</script>

<Card>
	<CardHeader>
		<CardTitle class="flex items-center gap-2"><SparklesIcon class="size-4" aria-hidden="true" />Similar books</CardTitle>
		<CardDescription>Suggestions are calculated from the visible local library. Provider search is only available in Edit metadata.</CardDescription>
	</CardHeader>
	<CardContent>
		{#if books.length === 0}
			<p class="rounded-lg border border-dashed p-5 text-sm text-muted-foreground">No similar books found in the visible library.</p>
		{:else}
			<div class="grid grid-cols-1 gap-2">
				{#each books as book (book.mediaId)}
					<a class="group flex items-center gap-3 rounded-lg border p-3 transition-colors hover:bg-muted/50" href={resolve('/(app)/book/[mediaId]', { mediaId: book.mediaId })}>
						<div class="flex size-10 shrink-0 items-center justify-center rounded-md bg-muted text-xs font-semibold" aria-hidden="true">{book.title.slice(0, 1).toUpperCase()}</div>
						<div class="min-w-0 flex-1">
							<p class="truncate text-sm font-medium group-hover:text-primary">{book.title}</p>
							<p class="truncate text-xs text-muted-foreground">{book.authors.join(', ') || 'Author unavailable'}</p>
							<div class="mt-1 flex items-center gap-2"><Badge variant="outline" class="text-[10px]">{labelKind(book.kind)}</Badge><span class="text-[10px] text-muted-foreground">{matchTier(book.score)}</span></div>
						</div>
						<ArrowUpRightIcon class="size-4 shrink-0 text-muted-foreground transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5" aria-hidden="true" />
					</a>
				{/each}
			</div>
		{/if}
	</CardContent>
</Card>
