<script lang="ts">
	import HighlighterIcon from '@lucide/svelte/icons/highlighter';
	import StickyNoteIcon from '@lucide/svelte/icons/sticky-note';
	import SmartphoneIcon from '@lucide/svelte/icons/smartphone';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import type { BookHighlight } from '$lib/book/detail';
	import { formatDate } from '$lib/book/detail';
	import { SOURCE_LABELS } from '$lib/annotations';

	let { highlights = [] }: { highlights?: BookHighlight[] } = $props();
</script>

<Card>
	<CardHeader>
		<CardTitle class="flex items-center gap-2"><HighlighterIcon class="size-4" aria-hidden="true" />Highlights & notes</CardTitle>
		<CardDescription>Annotations from every linked edition, with their originating protocol and device.</CardDescription>
	</CardHeader>
	<CardContent>
		{#if highlights.length === 0}
			<p class="rounded-lg border border-dashed p-5 text-sm text-muted-foreground">No highlights or notes have been recorded for this work.</p>
		{:else}
			<div class="grid gap-3">
				{#each highlights as highlight (highlight.id)}
					<article class="rounded-xl border p-4">
						<div class="flex flex-wrap items-center gap-2 text-xs">
							<Badge variant={highlight.kind === 'NOTE' ? 'secondary' : 'outline'}>{highlight.kind.replaceAll('_', ' ')}</Badge>
							<Badge variant="outline">{highlight.source}</Badge>
							{#if highlight.editable}<Badge variant="secondary">Editable</Badge>{/if}
							<span class="ml-auto text-muted-foreground">{formatDate(highlight.createdAt)}</span>
						</div>
						{#if highlight.lastEditedSource && highlight.lastEditedSource !== highlight.source}
							<p class="mt-2 text-xs text-muted-foreground">
								Edited in {SOURCE_LABELS[highlight.lastEditedSource as keyof typeof SOURCE_LABELS]}
								{#if highlight.lastEditedAt} · {formatDate(highlight.lastEditedAt)}{/if}
							</p>
						{/if}
						{#if highlight.excerpt}<blockquote class="mt-3 border-l-2 pl-3 text-sm leading-6">{highlight.excerpt}</blockquote>{/if}
						{#if highlight.note}<p class="mt-3 flex items-start gap-2 text-sm text-muted-foreground"><StickyNoteIcon class="mt-0.5 size-4 shrink-0" aria-hidden="true" />{highlight.note}</p>{/if}
						<div class="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
							{#if highlight.chapterTitle}<span>Chapter: {highlight.chapterTitle}</span>{/if}
							{#if highlight.page != null}<span>Page {highlight.page}</span>{/if}
							{#if highlight.progression != null}<span>{Math.round(highlight.progression * 100)}%</span>{/if}
							<span class="inline-flex items-center gap-1"><SmartphoneIcon class="size-3" aria-hidden="true" />{highlight.sourceDeviceName ?? highlight.sourceDeviceId ?? 'Device unavailable'}</span>
						</div>
						{#if highlight.href}<p class="mt-3 truncate font-mono text-xs text-muted-foreground" title={highlight.href}>Location: {highlight.href}{highlight.fragment ? `#${highlight.fragment}` : ''}</p>{/if}
					</article>
				{/each}
			</div>
		{/if}
	</CardContent>
</Card>
