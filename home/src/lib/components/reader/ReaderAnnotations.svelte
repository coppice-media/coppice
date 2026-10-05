<script lang="ts" module>
	import type {
		AnnotationKind,
		DeviceKind,
		ReadiumLocatorInput
	} from '$lib/graphql/generated/graphql';
	import type { ReaderLocator } from './locator';

	/** One `annotations` row for this book, joined to its native locator. */
	export type ReaderAnnotationRecord = {
		id: string;
		kind: AnnotationKind;
		source: DeviceKind;
		sourceDeviceId: string | null;
		sourceDeviceName: string | null;
		revision: number | null;
		lastEditedSource: DeviceKind | null;
		lastEditedAt: string | null;
		editable: boolean;
		chapterTitle: string | null;
		locator: ReaderLocator | null;
		href: string | null;
		fragment: string | null;
		page: number | null;
		progression: number | null;
		/** A moment in an audiobook: milliseconds from the start of the publication. */
		positionMs: number | null;
		excerpt: string | null;
		note: string | null;
		color: string | null;
	};

	/**
	 * An annotation about to be created: a selected passage (`excerpt` set,
	 * saved as a highlight with an optional note) or a place (`excerpt` null,
	 * saved as a note that needs text). A place is a page `locator`, or in an
	 * audiobook a `positionMs` with no locator; exactly one of the two is set.
	 */
	export type ReaderAnnotationDraft = {
		/** The book the anchor belongs to; a draft from another book is ignored. */
		mediaId: string;
		locator: ReadiumLocatorInput | null;
		positionMs: number | null;
		excerpt: string | null;
		/** Where the draft is anchored, for the composer heading. */
		label: string | null;
	};
</script>

<script lang="ts">
	/**
	 * The account reader's annotation panel: every annotation the user has on
	 * this book, with authoring for the rows the server marks `editable`.
	 *
	 * Creation goes through `createAnnotation`: a native `media_annotations`
	 * row anchored by the locator the reader produced, or in an audiobook by
	 * the player's publication time and no locator. Edits and deletes route by
	 * id exactly as the annotation hub does and send the row's `revision` as
	 * `expectedRevision`, so a stale edit of a Liseur-backed record is refused;
	 * on that conflict the list is refetched and the user told. Colours come
	 * from the shared `ANNOTATION_COLORS` picker, the one vocabulary both
	 * native and Liseur-backed rows accept.
	 */
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import LocateIcon from '@lucide/svelte/icons/locate';
	import LockIcon from '@lucide/svelte/icons/lock';
	import StickyNoteIcon from '@lucide/svelte/icons/sticky-note';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import XIcon from '@lucide/svelte/icons/x';
	import { toast } from 'svelte-sonner';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Textarea } from '@stump/ui/components/ui/textarea';
	import { request } from '@stump/ui/graphql/client';
	import {
		ReaderCreateAnnotationDocument,
		ReaderDeleteAnnotationDocument,
		ReaderUpdateAnnotationDocument,
		type CreateAnnotationInput,
		type UpdateAnnotationInput
	} from '$lib/graphql/generated/graphql';
	import { KIND_LABELS, SOURCE_LABELS, anchorLabel } from '$lib/annotations';
	import AnnotationColorPicker from '$lib/components/annotations/AnnotationColorPicker.svelte';
	import { absoluteTime, clockLabel, relativeTime } from '$lib/format';

	let {
		mediaId,
		annotations,
		draft = $bindable(null),
		selectable = false,
		notePage = null,
		pageCount = 0,
		noteTimeMs = null,
		durationMs = 0,
		jumpable,
		onjump
	}: {
		mediaId: string;
		annotations: ReaderAnnotationRecord[];
		draft?: ReaderAnnotationDraft | null;
		/** The reader turns text selections into drafts (EPUB). */
		selectable?: boolean;
		/** The visible page a page note would anchor to (comics and PDFs). */
		notePage?: number | null;
		pageCount?: number;
		/** The playhead a time note would anchor to (audiobooks), in publication ms. */
		noteTimeMs?: number | null;
		durationMs?: number;
		/** Ids the reader can scroll to. */
		jumpable: ReadonlySet<string>;
		onjump: (id: string) => void;
	} = $props();

	const queryClient = useQueryClient();

	// Reading order: whole-publication progression where the anchor has one,
	// otherwise the page or the moment in the recording, and unanchored rows
	// last in server order.
	const ordered = $derived(
		annotations
			.map((annotation, index) => ({ annotation, index }))
			.sort((left, right) => {
				const key = ({ annotation }: { annotation: ReaderAnnotationRecord }) =>
					annotation.progression ??
					(annotation.page !== null && pageCount > 0
						? annotation.page / pageCount
						: annotation.positionMs !== null && durationMs > 0
							? annotation.positionMs / durationMs
							: Number.POSITIVE_INFINITY);
				return key(left) - key(right) || left.index - right.index;
			})
			.map(({ annotation }) => annotation)
	);

	const activeDraft = $derived(draft?.mediaId === mediaId ? draft : null);
	let draftNote = $state('');
	let editingId = $state<string | null>(null);
	let editNote = $state('');
	let editColor = $state<string | null>(null);
	let editError = $state<string | null>(null);
	let savingId = $state<string | null>(null);
	let deletingId = $state<string | null>(null);

	function message(error: unknown, fallback: string): string {
		return error instanceof Error ? error.message : fallback;
	}

	function isRevisionConflict(error: unknown): boolean {
		return error instanceof Error && /annotation revision conflict/i.test(error.message);
	}

	/** Refetch this reader's rows (awaited) and stale every other annotation list. */
	async function refresh(): Promise<void> {
		void queryClient.invalidateQueries({ queryKey: ['annotations'] });
		void queryClient.invalidateQueries({ queryKey: ['annotation-books'] });
		void queryClient.invalidateQueries({ queryKey: ['book-highlights'] });
		await queryClient.invalidateQueries({ queryKey: ['readerAnnotations', mediaId] });
	}

	async function onWriteError(error: unknown, fallback: string): Promise<void> {
		await refresh();
		if (isRevisionConflict(error)) {
			editingId = null;
			toast.error(
				'This annotation was changed in another app. Reloaded the latest version — review it and try again.'
			);
			return;
		}
		toast.error(message(error, fallback));
	}

	const create = createMutation(() => ({
		mutationFn: (variables: { input: CreateAnnotationInput; submitted: ReaderAnnotationDraft }) =>
			request(ReaderCreateAnnotationDocument, { input: variables.input }),
		onSuccess: async (_data, { submitted }) => {
			await refresh();
			// A passage selected while the request was in flight is a new
			// draft and stays open.
			if (draft === submitted) {
				draft = null;
				draftNote = '';
			}
			toast.success(submitted.excerpt ? 'Highlight saved.' : 'Note saved.');
		},
		onError: (error) => toast.error(message(error, 'The annotation could not be saved.'))
	}));

	const update = createMutation(() => ({
		mutationFn: (input: UpdateAnnotationInput) =>
			request(ReaderUpdateAnnotationDocument, { input }),
		onSuccess: async () => {
			await refresh();
			editingId = null;
			toast.success('Annotation saved.');
		},
		onError: (error) => onWriteError(error, 'The annotation could not be saved.'),
		onSettled: () => (savingId = null)
	}));

	const remove = createMutation(() => ({
		mutationFn: (variables: { id: string; expectedRevision: number | null }) =>
			request(ReaderDeleteAnnotationDocument, variables),
		onSuccess: async (_data, { id }) => {
			await refresh();
			if (editingId === id) editingId = null;
			toast.success('Annotation deleted.');
		},
		onError: (error) => onWriteError(error, 'The annotation could not be deleted.'),
		onSettled: () => (deletingId = null)
	}));

	function submitDraft(event: SubmitEvent): void {
		event.preventDefault();
		const submitted = activeDraft;
		if (!submitted || create.isPending) return;
		const note = draftNote.trim();
		if (!submitted.excerpt && !note) return;
		create.mutate({
			input: {
				mediaId: submitted.mediaId,
				locator: submitted.locator,
				positionMs: submitted.positionMs,
				annotationText: note || null
			},
			submitted
		});
	}

	function cancelDraft(): void {
		draft = null;
		draftNote = '';
	}

	// A page note anchors through `locations.position`, the visible page the
	// paged reader draws and the hub's `?page=` deep link reopens.
	function startNote(page: number): void {
		draft = {
			mediaId,
			label: `Page ${page}`,
			excerpt: null,
			locator: {
				href: '',
				locations: {
					position: page,
					totalProgression: pageCount > 0 ? page / pageCount : null
				}
			},
			positionMs: null
		};
	}

	// A recording has no Readium anchor: a time note is anchored by the
	// playhead alone, in publication milliseconds, which "Go to" seeks back to.
	function startTimeNote(positionMs: number): void {
		draft = {
			mediaId,
			label: clockLabel(positionMs),
			excerpt: null,
			locator: null,
			positionMs
		};
	}

	function startEditing(annotation: ReaderAnnotationRecord): void {
		editingId = annotation.id;
		editNote = annotation.note ?? '';
		editColor = annotation.color;
		editError = null;
	}

	function save(annotation: ReaderAnnotationRecord): void {
		const note = editNote.trim();
		// A revisioned note is CAS-backed, and that lane refuses an empty body.
		if (annotation.kind === 'NOTE' && annotation.revision !== null && !note) {
			editError = 'This note needs text. Delete it instead to remove it.';
			return;
		}
		// Only highlights carry a colour: Liseur refuses one on a note.
		const color = annotation.kind === 'HIGHLIGHT' ? (editColor ?? '') : null;
		if (note === (annotation.note ?? '') && (color === null || color === (annotation.color ?? ''))) {
			editingId = null;
			return;
		}
		editError = null;
		savingId = annotation.id;
		// An empty string clears the note on both lanes; null would leave a
		// Liseur body untouched.
		update.mutate({
			id: annotation.id,
			annotationText: note,
			color,
			expectedRevision: annotation.revision
		});
	}

	function deleteAnnotation(annotation: ReaderAnnotationRecord): void {
		deletingId = annotation.id;
		remove.mutate({ id: annotation.id, expectedRevision: annotation.revision });
	}

	function ownerLabel(annotation: ReaderAnnotationRecord): string {
		return annotation.sourceDeviceName ?? SOURCE_LABELS[annotation.source];
	}
</script>

{#if annotations.length || selectable || notePage !== null || noteTimeMs !== null}
	<Card>
		<CardHeader class="flex flex-row flex-wrap items-start gap-3">
			<div class="mr-auto flex flex-col gap-1">
				<CardTitle class="text-base">Annotations</CardTitle>
				<p class="text-sm text-muted-foreground">
					{#if selectable}
						Select text in the book to highlight it. Highlights and notes from your devices appear
						here too; the ones they own stay read-only.
					{:else if notePage !== null}
						Add a note to the page you are on. Annotations from your devices appear here too; the
						ones they own stay read-only.
					{:else if noteTimeMs !== null}
						Add a note at the moment you are listening to. Annotations from your devices appear
						here too; the ones they own stay read-only.
					{:else}
						Annotations on this book from every source. The ones a device owns stay read-only.
					{/if}
				</p>
			</div>
			{#if notePage !== null}
				<Button size="sm" variant="outline" onclick={() => startNote(notePage)}>
					<StickyNoteIcon data-icon="inline-start" />
					Note on page {notePage}
				</Button>
			{/if}
			{#if noteTimeMs !== null}
				<Button size="sm" variant="outline" onclick={() => startTimeNote(noteTimeMs)}>
					<StickyNoteIcon data-icon="inline-start" />
					Note at {clockLabel(noteTimeMs)}
				</Button>
			{/if}
		</CardHeader>
		<CardContent class="px-0">
			{#if ordered.length === 0}
				<p class="border-t px-6 py-4 text-sm text-muted-foreground">No annotations on this book yet.</p>
			{:else}
				<ul class="flex flex-col">
					{#each ordered as annotation (annotation.id)}
						{@const anchor = anchorLabel(annotation)}
						<li
							class={[
								'flex flex-col gap-2 border-t px-6 py-3',
								!annotation.editable && 'bg-muted/40'
							]}
						>
							<div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
								<Badge variant="secondary">{KIND_LABELS[annotation.kind]}</Badge>
								<Badge variant="outline" title={ownerLabel(annotation)}>
									{ownerLabel(annotation)}
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
									<span class="ml-auto">{anchor}</span>
								{/if}
							</div>
							{#if annotation.lastEditedSource && annotation.lastEditedSource !== annotation.source}
								<p class="text-xs text-muted-foreground">
									Edited in {SOURCE_LABELS[annotation.lastEditedSource]}
									{#if annotation.lastEditedAt}
										· <time
											datetime={annotation.lastEditedAt}
											title={absoluteTime(annotation.lastEditedAt)}
										>
											{relativeTime(annotation.lastEditedAt)}
										</time>
									{/if}
								</p>
							{/if}
							{#if annotation.excerpt}
								<blockquote class="border-l-2 border-primary/50 pl-3 text-sm">
									{annotation.excerpt}
								</blockquote>
							{/if}

							{#if editingId === annotation.id}
								<form
									class="flex flex-col gap-2"
									onsubmit={(event) => {
										event.preventDefault();
										save(annotation);
									}}
								>
									<Textarea
										bind:value={editNote}
										rows={3}
										aria-label="Note"
										placeholder={annotation.kind === 'NOTE'
											? 'Your note'
											: 'Your note on this passage (optional)'}
									/>
									{#if annotation.kind === 'HIGHLIGHT'}
										<AnnotationColorPicker bind:value={editColor} />
									{/if}
									{#if editError}
										<p class="text-sm text-destructive">{editError}</p>
									{/if}
									<div class="flex gap-2">
										<Button type="submit" size="sm" disabled={savingId === annotation.id}>
											{savingId === annotation.id ? 'Saving…' : 'Save'}
										</Button>
										<Button
											type="button"
											size="sm"
											variant="ghost"
											onclick={() => (editingId = null)}
										>
											Cancel
										</Button>
									</div>
								</form>
							{:else if annotation.note}
								<p class="text-sm whitespace-pre-wrap">{annotation.note}</p>
							{:else if !annotation.excerpt}
								<p class="text-sm text-muted-foreground">No text — this is a place marker.</p>
							{/if}

							<div class="flex flex-wrap items-center gap-2">
								{#if jumpable.has(annotation.id)}
									<Button size="sm" variant="outline" onclick={() => onjump(annotation.id)}>
										<LocateIcon data-icon="inline-start" />
										Go to
									</Button>
								{/if}
								{#if annotation.editable}
									{#if editingId !== annotation.id}
										<Button size="sm" variant="ghost" onclick={() => startEditing(annotation)}>
											{annotation.kind === 'HIGHLIGHT' ? 'Edit' : 'Edit note'}
										</Button>
									{/if}
									<Button
										size="sm"
										variant="ghost"
										class="text-destructive"
										disabled={deletingId === annotation.id}
										onclick={() => deleteAnnotation(annotation)}
									>
										<Trash2Icon data-icon="inline-start" />
										{deletingId === annotation.id ? 'Deleting…' : 'Delete'}
									</Button>
								{:else}
									<span class="text-xs text-muted-foreground">
										{annotation.kind === 'BOOKMARK'
											? 'Bookmarks cannot be edited here.'
											: `Owned by ${ownerLabel(annotation)} — edit it there and it syncs back.`}
									</span>
								{/if}
							</div>
						</li>
					{/each}
				</ul>
			{/if}
		</CardContent>
	</Card>
{/if}

<svelte:window
	onkeydown={(event) => {
		if (activeDraft && event.key === 'Escape') cancelDraft();
	}}
/>

{#if activeDraft}
	<form
		class="fixed inset-x-4 bottom-4 z-40 mx-auto flex max-w-xl flex-col gap-3 rounded-xl border bg-popover p-4 text-popover-foreground shadow-lg"
		aria-label={activeDraft.excerpt ? 'New highlight' : 'New note'}
		onsubmit={submitDraft}
	>
		<div class="flex items-start gap-2">
			<h2 class="mr-auto text-sm font-medium">
				{activeDraft.excerpt ? 'Highlight this passage' : 'New note'}
				{#if activeDraft.label}
					<span class="font-normal text-muted-foreground">· {activeDraft.label}</span>
				{/if}
			</h2>
			<Button type="button" size="icon-sm" variant="ghost" aria-label="Discard" onclick={cancelDraft}>
				<XIcon />
			</Button>
		</div>
		{#if activeDraft.excerpt}
			<blockquote class="line-clamp-3 border-l-2 border-primary/50 pl-3 text-sm">
				{activeDraft.excerpt}
			</blockquote>
		{/if}
		<Textarea
			bind:value={draftNote}
			rows={2}
			aria-label="Note"
			required={!activeDraft.excerpt}
			placeholder={activeDraft.excerpt ? 'Add a note (optional)' : 'Your note'}
		/>
		<div class="flex gap-2">
			<Button
				type="submit"
				size="sm"
				disabled={create.isPending || (!activeDraft.excerpt && !draftNote.trim())}
			>
				{create.isPending ? 'Saving…' : activeDraft.excerpt ? 'Save highlight' : 'Save note'}
			</Button>
			<Button type="button" size="sm" variant="ghost" onclick={cancelDraft}>Cancel</Button>
		</div>
	</form>
{/if}
