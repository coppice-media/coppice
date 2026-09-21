<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import * as Dialog from '@stump/ui/components/ui/dialog';
	import { request } from '@stump/ui/graphql/client';
	import {
		ReaderChapterMapDocument,
		ReaderChapterMapMediaDocument,
		ReaderClearChapterMapEntryDocument,
		ReaderSetChapterMapEntryDocument
	} from '$lib/graphql/generated/graphql';

	type SpineItem = { idref: string; id: string | null; linear: boolean };
	type AudioChapter = { index: number; title: string | null };
	type Props = {
		open?: boolean;
		ebookMediaId: string;
		audioMediaId: string;
	};

	let { open = $bindable(false), ebookMediaId, audioMediaId }: Props = $props();

	const queryClient = useQueryClient();
	let drafts = $state<Record<number, number | null>>({});

	const mapQuery = createQuery(() => ({
		queryKey: ['readerChapterMap', ebookMediaId, audioMediaId],
		queryFn: () => request(ReaderChapterMapDocument, { ebookMediaId, audioMediaId }),
		enabled: browser && open && ebookMediaId.length > 0 && audioMediaId.length > 0
	}));
	const sourceQuery = createQuery(() => ({
		queryKey: ['readerChapterMapMedia', ebookMediaId, audioMediaId],
		queryFn: async () => {
			const [ebookResult, audioResult] = await Promise.all([
				request(ReaderChapterMapMediaDocument, { id: ebookMediaId }),
				request(ReaderChapterMapMediaDocument, { id: audioMediaId })
			]);
			const spine = ebookResult.mediaById?.ebook?.spine ?? [];
			return {
				spineItems: spine.filter((item) => item.linear),
				audioChapters: audioResult.mediaById?.audio?.chapters ?? []
			};
		},
		enabled: browser && open && ebookMediaId.length > 0 && audioMediaId.length > 0
	}));
	const entries = $derived(mapQuery.data?.chapterMap ?? []);
	const spineItems = $derived(sourceQuery.data?.spineItems ?? []);
	const audioChapters = $derived(sourceQuery.data?.audioChapters ?? []);
	const entryBySpine = $derived(
		new Map(entries.map((entry) => [entry.ebookSpineIndex, entry]))
	);
	const rows = $derived(
		spineItems.map((_, index) => ({
			index,
			entry: entryBySpine.get(index),
			selected: drafts[index] ?? entryBySpine.get(index)?.audioChapterIndex ?? null
		}))
	);

	// Load server values when the dialog opens or the map is refreshed. This
	// deliberately does not depend on drafts, so changing a select is local
	// until its row is saved.
	$effect(() => {
		if (!open) {
			drafts = {};
			return;
		}
		const next: Record<number, number | null> = {};
		for (const [index, entry] of entryBySpine) next[index] = entry.audioChapterIndex;
		drafts = next;
	});

	function invalidate(): void {
		void queryClient.invalidateQueries({ queryKey: ['readerChapterMap'] });
		void queryClient.invalidateQueries({ queryKey: ['readerEditions'] });
	}

	function errorMessage(error: unknown, fallback: string): string {
		return error instanceof Error ? error.message : fallback;
	}

	const saveMutation = createMutation(() => ({
		mutationFn: ({ spineIndex, audioIndex }: { spineIndex: number; audioIndex: number }) =>
			request(ReaderSetChapterMapEntryDocument, {
				ebookMediaId,
				audioMediaId,
				ebookSpineIndex: spineIndex,
				audioChapterIndex: audioIndex,
				confidence: 1.0
			}),
		onSuccess: () => {
			invalidate();
			toast.success('Chapter map entry saved.');
		},
		onError: (error) => toast.error(errorMessage(error, 'Unable to save the chapter map entry.'))
	}));

	const clearMutation = createMutation(() => ({
		mutationFn: (spineIndex: number) =>
			request(ReaderClearChapterMapEntryDocument, {
				ebookMediaId,
				audioMediaId,
				ebookSpineIndex: spineIndex
			}),
		onSuccess: () => {
			invalidate();
			toast.success('Chapter map entry cleared.');
		},
		onError: (error) => toast.error(errorMessage(error, 'Unable to clear the chapter map entry.'))
	}));

	function selectChapter(spineIndex: number, event: Event): void {
		const value = (event.currentTarget as HTMLSelectElement).value;
		drafts[spineIndex] = value === '' ? null : Number(value);
	}

	function saveRow(spineIndex: number): void {
		const audioIndex = drafts[spineIndex] ?? entryBySpine.get(spineIndex)?.audioChapterIndex;
		if (audioIndex == null || !Number.isInteger(audioIndex)) return;
		saveMutation.mutate({ spineIndex, audioIndex });
	}

	function clearRow(spineIndex: number): void {
		clearMutation.mutate(spineIndex);
	}

	function spineLabel(index: number): string {
		return `Spine item ${index}`;
	}

	function audioLabel(chapter: AudioChapter): string {
		const title = chapter.title?.trim();
		return title ? `${chapter.index} · ${title}` : `Audio chapter ${chapter.index}`;
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="flex max-h-[90vh] flex-col overflow-hidden sm:max-w-4xl">
		<Dialog.Header>
			<Dialog.Title>Chapter map</Dialog.Title>
			<Dialog.Description>
				Choose the audiobook chapter corresponding to each linear EPUB spine item. Manual edits are
				saved with full confidence.
			</Dialog.Description>
		</Dialog.Header>

		{#if mapQuery.isPending || sourceQuery.isPending}
			<p class="py-8 text-center text-sm text-muted-foreground">Loading chapter map…</p>
		{:else if mapQuery.error || sourceQuery.error}
			<p class="py-8 text-center text-sm text-destructive">
				Unable to load the chapter map and source chapters.
			</p>
		{:else if spineItems.length === 0}
			<p class="py-8 text-center text-sm text-muted-foreground">
				No linear EPUB spine items are available.
			</p>
		{:else}
			<div class="min-h-0 overflow-auto rounded-md border">
				<table class="w-full text-sm">
					<thead class="sticky top-0 bg-muted/95 text-left text-xs text-muted-foreground">
						<tr>
							<th class="px-3 py-2 font-medium">Ebook spine</th>
							<th class="px-3 py-2 font-medium">Audio chapter</th>
							<th class="px-3 py-2 font-medium">Confidence</th>
							<th class="px-3 py-2 text-right font-medium">Actions</th>
						</tr>
					</thead>
					<tbody class="divide-y">
						{#each rows as row (row.index)}
							<tr>
								<td class="px-3 py-2 align-middle font-medium">{spineLabel(row.index)}</td>
								<td class="min-w-56 px-3 py-2 align-middle">
									<select
										class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
										aria-label={`Audio chapter for ${spineLabel(row.index)}`}
										value={row.selected ?? ''}
										onchange={(event) => selectChapter(row.index, event)}
										disabled={audioChapters.length === 0}
									>
										<option value="">Not mapped</option>
										{#each audioChapters as chapter (chapter.index)}
											<option value={chapter.index}>{audioLabel(chapter)}</option>
										{/each}
									</select>
								</td>
								<td class="px-3 py-2 align-middle">
									{#if row.entry}
										<Badge variant={row.entry.confidence >= 0.99 ? 'default' : 'secondary'}>
											{Math.round(row.entry.confidence * 100)}% confidence
										</Badge>
									{:else}
										<Badge variant="outline">Unmapped</Badge>
									{/if}
								</td>
								<td class="px-3 py-2 align-middle">
									<div class="flex justify-end gap-1">
										<Button
											size="xs"
											onclick={() => saveRow(row.index)}
											disabled={saveMutation.isPending || row.selected == null || audioChapters.length === 0}
										>
											Save
										</Button>
										{#if row.entry}
											<Button
												size="xs"
												variant="ghost"
												onclick={() => clearRow(row.index)}
												disabled={clearMutation.isPending}
											>
												Clear
											</Button>
										{/if}
									</div>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}

		{#if audioChapters.length === 0}
			<p class="text-xs text-muted-foreground">This audiobook has no chapter marks to map.</p>
		{/if}
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (open = false)}>Done</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
