<script lang="ts">
	/**
	 * One highlight, note, or bookmark.
	 *
	 * The note is editable in place, but only for native rows: a liseur-sync
	 * CAS record is owned by the device that pushed it and is replicated with
	 * compare-and-set revisions, so the server reports `editable: false` and
	 * this card shows it read-only.
	 */
	import { resolve } from '$app/paths';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import BookmarkIcon from '@lucide/svelte/icons/bookmark';
	import HighlighterIcon from '@lucide/svelte/icons/highlighter';
	import StickyNoteIcon from '@lucide/svelte/icons/sticky-note';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Textarea } from '@stump/ui/components/ui/textarea';
	import type { ConsoleAnnotationFieldsFragment } from '$lib/graphql/generated/graphql';
	import { KIND_LABELS, SOURCE_LABELS, anchorLabel, readerAnchor } from '$lib/annotations';
	import { relativeTime } from '$lib/format';

	let {
		annotation,
		saving = false,
		deleting = false,
		onsave,
		ondelete
	}: {
		annotation: ConsoleAnnotationFieldsFragment;
		saving?: boolean;
		deleting?: boolean;
		onsave: (id: string, note: string | null) => void;
		ondelete: (id: string) => void;
	} = $props();

	const KIND_ICONS = {
		HIGHLIGHT: HighlighterIcon,
		NOTE: StickyNoteIcon,
		BOOKMARK: BookmarkIcon
	};
	const Icon = $derived(KIND_ICONS[annotation.kind]);
	const anchor = $derived(anchorLabel(annotation));
	const readerHref = $derived(
		annotation.book.mediaId
			? `${resolve('/(app)/reader/[mediaId]', { mediaId: annotation.book.mediaId })}${readerAnchor(annotation)}`
			: null
	);

	let editing = $state(false);
	let draft = $state('');

	function startEditing(): void {
		draft = annotation.note ?? '';
		editing = true;
	}

	function save(): void {
		const next = draft.trim();
		editing = false;
		if (next === (annotation.note ?? '')) return;
		onsave(annotation.id, next ? next : null);
	}
</script>

<li class="flex flex-col gap-2 border-t px-4 py-3 first:border-t-0">
	<div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
		<Badge variant="secondary" class="gap-1">
			<Icon class="size-3" aria-hidden="true" />
			{KIND_LABELS[annotation.kind]}
		</Badge>
		<Badge variant="outline" title={annotation.sourceDeviceName ?? undefined}>
			{SOURCE_LABELS[annotation.source]}
		</Badge>
		{#if annotation.color}
			<span
				class="size-3 rounded-full border"
				style={`background-color: ${annotation.color}`}
				title={`Colour: ${annotation.color}`}
			></span>
		{/if}
		{#if anchor}
			<span>{anchor}</span>
		{/if}
		<span class="ml-auto">{relativeTime(annotation.createdAt)}</span>
	</div>

	{#if annotation.excerpt}
		<blockquote class="border-l-2 pl-3 text-sm italic">{annotation.excerpt}</blockquote>
	{/if}

	{#if editing}
		<div class="flex flex-col gap-2">
			<Textarea
				bind:value={draft}
				rows={3}
				aria-label="Note"
				placeholder="Your note on this passage"
			/>
			<div class="flex gap-2">
				<Button size="sm" disabled={saving} onclick={save}>
					{saving ? 'Saving…' : 'Save note'}
				</Button>
				<Button size="sm" variant="ghost" onclick={() => (editing = false)}>Cancel</Button>
			</div>
		</div>
	{:else if annotation.note}
		<p class="text-sm whitespace-pre-wrap">{annotation.note}</p>
	{:else if !annotation.excerpt}
		<p class="text-sm text-muted-foreground">No text — this is a place marker.</p>
	{/if}

	<div class="flex flex-wrap items-center gap-2">
		{#if readerHref}
			<Button size="sm" variant="outline" href={readerHref}>
				<BookOpenIcon aria-hidden="true" />
				Open in reader
			</Button>
		{/if}
		{#if annotation.editable}
			{#if !editing}
				<Button size="sm" variant="ghost" onclick={startEditing}>
					{annotation.note ? 'Edit note' : 'Add note'}
				</Button>
			{/if}
			<Button
				size="sm"
				variant="ghost"
				class="text-destructive"
				disabled={deleting}
				onclick={() => ondelete(annotation.id)}
			>
				<Trash2Icon aria-hidden="true" />
				{deleting ? 'Deleting…' : 'Delete'}
			</Button>
		{:else}
			<span class="text-xs text-muted-foreground">
				{annotation.source === 'WEB'
					? 'Bookmarks are managed in the reader.'
					: `Owned by ${annotation.sourceDeviceName ?? SOURCE_LABELS[annotation.source]} — edit it there and it syncs back.`}
			</span>
		{/if}
	</div>
</li>
