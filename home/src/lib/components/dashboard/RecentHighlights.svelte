<script lang="ts">
	/**
	 * The newest highlights, notes, and bookmarks across every book and
	 * source, one row each: the passage (or note), the book, and the device
	 * the annotation came from.
	 */
	import { resolve } from '$app/paths';
	import BookmarkIcon from '@lucide/svelte/icons/bookmark';
	import HighlighterIcon from '@lucide/svelte/icons/highlighter';
	import StickyNoteIcon from '@lucide/svelte/icons/sticky-note';
	import { Badge } from '@stump/ui/components/ui/badge';
	import type { DashboardRecentAnnotationsQuery } from '$lib/graphql/generated/graphql';
	import { KIND_LABELS, SOURCE_LABELS } from '$lib/annotations';
	import { DEVICE_KIND_ICONS } from '$lib/devices';
	import { absoluteTime, relativeTime } from '$lib/format';
	import Widget from './Widget.svelte';

	type Annotation = DashboardRecentAnnotationsQuery['annotations']['items'][number];

	const KIND_ICONS = {
		HIGHLIGHT: HighlighterIcon,
		NOTE: StickyNoteIcon,
		BOOKMARK: BookmarkIcon
	};

	let {
		annotations,
		total,
		query,
		now = new Date(),
		class: className
	}: {
		annotations: readonly Annotation[];
		total: number;
		query: { isPending: boolean; error: unknown; refetch: () => unknown };
		now?: Date;
		class?: string;
	} = $props();
</script>

<Widget
	title="Recent highlights"
	description={total ? `${total.toLocaleString()} in total, across every device.` : undefined}
	href={resolve('/annotations')}
	{query}
	empty={annotations.length === 0}
	emptyTitle="No highlights yet"
	emptyDescription="Select a passage in the reader, or sync a device that carries annotations."
	errorTitle="Unable to load annotations"
	rows={5}
	class={className}
>
	<ul class="-my-1 flex flex-col divide-y">
		{#each annotations as annotation (annotation.id)}
			{@const Icon = KIND_ICONS[annotation.kind]}
			{@const SourceIcon = DEVICE_KIND_ICONS[annotation.source]}
			<li class="flex items-start gap-3 py-3">
				<span
					class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
					title={KIND_LABELS[annotation.kind]}
				>
					<Icon class="size-4" aria-hidden="true" />
					<span class="sr-only">{KIND_LABELS[annotation.kind]}</span>
				</span>
				<div class="flex min-w-0 flex-1 flex-col gap-1">
					{#if annotation.excerpt}
						<p class="line-clamp-2 text-sm leading-snug">{annotation.excerpt}</p>
						{#if annotation.note}
							<p class="line-clamp-1 text-xs text-muted-foreground">{annotation.note}</p>
						{/if}
					{:else if annotation.note}
						<p class="line-clamp-2 text-sm leading-snug">{annotation.note}</p>
					{:else}
						<p class="text-sm text-muted-foreground">
							{annotation.chapterTitle ??
								(annotation.page ? `Page ${annotation.page}` : 'A place marker')}
						</p>
					{/if}
					<div class="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
						<a
							class="min-w-0 truncate font-medium text-foreground hover:underline"
							href={annotation.book.mediaId
								? `${resolve('/annotations')}?book=${encodeURIComponent(annotation.book.mediaId)}`
								: resolve('/annotations')}
						>
							{annotation.book.title}
						</a>
						<Badge variant="outline" title={SOURCE_LABELS[annotation.source]}>
							<SourceIcon aria-hidden="true" />
							{annotation.sourceDeviceName ?? SOURCE_LABELS[annotation.source]}
						</Badge>
						{#if annotation.lastEditedSource && annotation.lastEditedSource !== annotation.source}
							<span title={annotation.lastEditedAt ? absoluteTime(annotation.lastEditedAt) : undefined}>
								Edited in {SOURCE_LABELS[annotation.lastEditedSource]}
							</span>
						{/if}
						<span class="ml-auto shrink-0" title={absoluteTime(annotation.createdAt)}>
							{relativeTime(annotation.createdAt, now)}
						</span>
					</div>
				</div>
			</li>
		{/each}
	</ul>
</Widget>
