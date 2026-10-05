<script lang="ts">
	/**
	 * One highlight, note, or bookmark. All entries marked editable by the
	 * server can be changed here; revisioned updates protect device-owned CAS
	 * records from stale writes.
	 *
	 * `onsave` sends what the server applies on every lane: `note` is `""` to
	 * clear (a Liseur body keeps its text on `null`), and `color` is `null` to
	 * leave it, `""` to clear, or an `ANNOTATION_COLORS` token. Only
	 * highlights carry a colour — Liseur refuses one on a note.
	 */
	import { resolve } from '$app/paths';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import BookmarkIcon from '@lucide/svelte/icons/bookmark';
	import HighlighterIcon from '@lucide/svelte/icons/highlighter';
	import LockIcon from '@lucide/svelte/icons/lock';
	import StickyNoteIcon from '@lucide/svelte/icons/sticky-note';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Textarea } from '@stump/ui/components/ui/textarea';
	import type { ConsoleAnnotationFieldsFragment } from '$lib/graphql/generated/graphql';
	import { KIND_LABELS, SOURCE_LABELS, anchorLabel, readerAnchor } from '$lib/annotations';
	import { DEVICE_KIND_ICONS } from '$lib/devices';
	import { absoluteTime, relativeTime } from '$lib/format';
	import AnnotationColorPicker from './AnnotationColorPicker.svelte';

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
		onsave: (id: string, note: string, color: string | null, expectedRevision: number | null) => void;
		ondelete: (id: string, expectedRevision: number | null) => void;
	} = $props();

	const KIND_ICONS = {
		HIGHLIGHT: HighlighterIcon,
		NOTE: StickyNoteIcon,
		BOOKMARK: BookmarkIcon
	};
	const Icon = $derived(KIND_ICONS[annotation.kind]);
	const SourceIcon = $derived(DEVICE_KIND_ICONS[annotation.source]);
	const anchor = $derived(anchorLabel(annotation));
	const readerHref = $derived(
		annotation.book.mediaId
			? `${resolve('/(app)/reader/[mediaId]', { mediaId: annotation.book.mediaId })}${readerAnchor(annotation)}`
			: null
	);

	let editing = $state(false);
	let draft = $state('');
	let draftColor = $state<string | null>(null);
	let editError = $state<string | null>(null);

	function startEditing(): void {
		draft = annotation.note ?? '';
		draftColor = annotation.color ?? null;
		editError = null;
		editing = true;
	}

	function save(): void {
		const next = draft.trim();
		// A revisioned note is CAS-backed, and that lane refuses an empty body.
		if (annotation.kind === 'NOTE' && annotation.revision != null && !next) {
			editError = 'This note needs text. Delete it instead to remove it.';
			return;
		}
		const nextColor = annotation.kind === 'HIGHLIGHT' ? (draftColor ?? '') : null;
		editing = false;
		if (
			next === (annotation.note ?? '') &&
			(nextColor === null || nextColor === (annotation.color ?? ''))
		)
			return;
		onsave(annotation.id, next, nextColor, annotation.revision);
	}
</script>

<li
	class="flex flex-col gap-3 border-t px-6 py-4 first:border-t-0 {annotation.editable
		? ''
		: 'bg-muted/40'}"
>
	<div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
		<Badge variant="secondary">
			<Icon aria-hidden="true" />
			{KIND_LABELS[annotation.kind]}
		</Badge>
		<Badge variant="outline" title={SOURCE_LABELS[annotation.source]}>
			<SourceIcon aria-hidden="true" />
			{annotation.sourceDeviceName ?? SOURCE_LABELS[annotation.source]}
		</Badge>
		{#if !annotation.editable}
			<Badge variant="outline" class="text-muted-foreground">
				<LockIcon aria-hidden="true" />
				Read-only
			</Badge>
		{/if}
		{#if annotation.color}
			<span
				class="size-3 rounded-full ring-1 ring-foreground/20"
				style:background-color={annotation.color}
				role="img"
				aria-label={`Colour: ${annotation.color}`}
				title={`Colour: ${annotation.color}`}
			></span>
		{/if}
		{#if anchor}
			<span>{anchor}</span>
		{/if}
		<time class="ml-auto" datetime={annotation.createdAt ?? undefined} title={absoluteTime(annotation.createdAt)}>
			{relativeTime(annotation.createdAt)}
		</time>
	</div>
	{#if annotation.lastEditedSource && annotation.lastEditedSource !== annotation.source}
		<p class="text-xs text-muted-foreground">
			Edited in {SOURCE_LABELS[annotation.lastEditedSource]}
			{#if annotation.lastEditedAt}
				· <time datetime={annotation.lastEditedAt} title={absoluteTime(annotation.lastEditedAt)}>
					{relativeTime(annotation.lastEditedAt)}
				</time>
			{/if}
		</p>
	{/if}

	{#if annotation.excerpt}
		<blockquote class="border-l-2 border-primary/50 pl-4 text-[0.9375rem] leading-relaxed text-foreground">
			{annotation.excerpt}
		</blockquote>
	{/if}

	{#if editing}
		<div class="flex flex-col gap-2">
			<Textarea
				bind:value={draft}
				rows={3}
				aria-label="Note"
				placeholder="Your note on this passage"
			/>
			{#if annotation.kind === 'HIGHLIGHT'}
				<AnnotationColorPicker bind:value={draftColor} />
			{/if}
			{#if editError}
				<p class="text-sm text-destructive">{editError}</p>
			{/if}
			<div class="flex gap-2">
				<Button size="sm" disabled={saving} onclick={save}>
					{saving ? 'Saving…' : 'Save note'}
				</Button>
				<Button size="sm" variant="ghost" onclick={() => (editing = false)}>Cancel</Button>
			</div>
		</div>
	{:else if annotation.note}
		<p class="flex gap-2 text-sm whitespace-pre-wrap text-muted-foreground">
			<StickyNoteIcon class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
			<span>{annotation.note}</span>
		</p>
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
				onclick={() => ondelete(annotation.id, annotation.revision)}
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
