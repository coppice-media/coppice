<script lang="ts">
	/**
	 * One book's annotations: a header identifying the book (with links into
	 * the library and the reader) over the list of its highlights, notes, and
	 * bookmarks.
	 */
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardHeader } from '@stump/ui/components/ui/card';
	import { countNoun } from '$lib/format';
	import type { AnnotationGroup } from '$lib/annotations';
	import AnnotationCard from './AnnotationCard.svelte';

	let {
		group,
		savingId = null,
		deletingId = null,
		onsave,
		ondelete
	}: {
		group: AnnotationGroup;
		savingId?: string | null;
		deletingId?: string | null;
		onsave: (id: string, note: string | null, color: string | null, expectedRevision: number | null) => void;
		ondelete: (id: string, expectedRevision: number | null) => void;
	} = $props();

	const { book } = $derived(group);
	// What the annotation-sync sinks name this book's file: `native:<id>` and
	// `liseur:<id>` are the same stable keys the export derives its file names
	// from, so the hub can name the file without asking the server.
	const exportFile = $derived(`${book.key.replace(':', '-')}.md`);
</script>

<Card>
	<CardHeader class="flex flex-wrap items-start gap-2">
		<div class="mr-auto">
			{#if book.mediaId}
				<a
					class="text-base font-semibold tracking-tight hover:underline"
					href={resolve('/(app)/reader/[mediaId]', { mediaId: book.mediaId })}
				>
					{book.title}
				</a>
			{:else}
				<span class="text-base font-semibold tracking-tight">{book.title}</span>
			{/if}
			<p class="text-xs text-muted-foreground">
				{#if book.authors.length}
					{book.authors.join(', ')} ·
				{/if}
				{countNoun(group.items.length, 'annotation')}
				{#if book.seriesName && book.seriesId}
					· <a
						class="hover:underline"
						href={resolve('/(app)/series/[id]', { id: book.seriesId })}>{book.seriesName}</a
					>
				{/if}
				·
				<a
					class="font-mono hover:underline"
					href={`${resolve('/connections')}#exports`}
					title="The file the annotation-sync sinks write this book to"
				>
					{exportFile}
				</a>
			</p>
		</div>
		{#if !book.mediaId}
			<Badge variant="outline" title="A liseur work with no book on this server">
				Not in your library
			</Badge>
		{:else if book.libraryId}
			<Button
				size="sm"
				variant="ghost"
				href={`${resolve('/(app)/library/[id]', { id: book.libraryId })}?tab=books`}
			>
				Library
			</Button>
		{/if}
	</CardHeader>
	<CardContent class="px-0">
		<ul class="flex flex-col">
			{#each group.items as annotation (annotation.id)}
				<AnnotationCard
					{annotation}
					saving={savingId === annotation.id}
					deleting={deletingId === annotation.id}
					{onsave}
					{ondelete}
				/>
			{/each}
		</ul>
	</CardContent>
</Card>
