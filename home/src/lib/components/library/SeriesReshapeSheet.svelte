<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Button } from '@stump/ui/components/ui/button';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import * as Select from '@stump/ui/components/ui/select';
	import { Separator } from '@stump/ui/components/ui/separator';
	import * as Sheet from '@stump/ui/components/ui/sheet';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleMergeSeriesDocument,
		ConsoleMoveMediaToSeriesDocument,
		ConsoleRenameSeriesDocument,
		ConsoleSeriesPickerDocument,
		ConsoleSplitSeriesDocument,
		type ConsoleSeriesDetailQuery,
		type SeriesMetadataInput
	} from '$lib/graphql/generated/graphql';
	import { countNoun } from '$lib/format';

	type SeriesDetail = NonNullable<ConsoleSeriesDetailQuery['seriesById']>;

	let {
		open = $bindable(false),
		series,
		selected,
		onreshaped
	}: {
		open?: boolean;
		series: SeriesDetail;
		/** The books ticked in the table; move and split act on these. */
		selected: string[];
		onreshaped: (clearSelection: boolean) => void;
	} = $props();

	const queryClient = useQueryClient();

	let title = $state('');
	let moveTarget = $state('');
	let mergeTarget = $state('');
	let splitName = $state('');

	$effect(() => {
		title = series.resolvedName;
	});

	// Every other series of the same library is a move or merge target.
	const siblingsQuery = createQuery(() => ({
		queryKey: ['seriesPicker', series.libraryId],
		queryFn: () =>
			request(ConsoleSeriesPickerDocument, {
				filter: { libraryId: { eq: series.libraryId ?? '' } }
			}),
		enabled: browser && open && !!series.libraryId
	}));
	const siblings = $derived(
		(siblingsQuery.data?.series.nodes ?? []).filter(
			(node) => node.id !== series.id && !node.sourceProvider
		)
	);
	const moveTargetLabel = $derived(
		siblings.find((node) => node.id === moveTarget)?.resolvedName ?? 'Pick a series'
	);
	const mergeTargetLabel = $derived(
		siblings.find((node) => node.id === mergeTarget)?.resolvedName ?? 'Pick a series'
	);

	function invalidate(): void {
		void queryClient.invalidateQueries({ queryKey: ['series'] });
		void queryClient.invalidateQueries({ queryKey: ['books'] });
		void queryClient.invalidateQueries({ queryKey: ['seriesPicker'] });
	}

	function failed(fallback: string) {
		return (error: unknown) =>
			toast.error(error instanceof Error ? error.message : fallback);
	}

	/**
	 * `updateSeriesMetadata` replaces every field of its input, so the rename
	 * writes back the metadata this screen read with only the title changed.
	 * The folder on disk keeps its name; `resolvedName` prefers the title.
	 */
	const rename = createMutation(() => ({
		mutationFn: () => {
			const metadata = series.metadata;
			const input: SeriesMetadataInput = {
				ageRating: metadata?.ageRating ?? null,
				booktype: metadata?.booktype ?? null,
				characters: metadata?.characters ?? null,
				collects:
					metadata?.collects.map((item) => ({
						series: item.series ?? null,
						comicid: item.comicid ?? null,
						issueid: item.issueid ?? null,
						issues: item.issues ?? null
					})) ?? null,
				comicImage: metadata?.comicImage ?? null,
				comicid: metadata?.comicid ?? null,
				descriptionFormatted: metadata?.descriptionFormatted ?? null,
				genres: metadata?.genres ?? null,
				imprint: metadata?.imprint ?? null,
				links: metadata?.links ?? null,
				metaType: metadata?.metaType ?? null,
				publicationRun: metadata?.publicationRun ?? null,
				publisher: metadata?.publisher ?? null,
				status: metadata?.status ?? null,
				summary: metadata?.summary ?? null,
				title: title.trim(),
				totalIssues: metadata?.totalIssues ?? null,
				volume: metadata?.volume ?? null,
				writers: metadata?.writers ?? null,
				year: metadata?.year ?? null
			};
			return request(ConsoleRenameSeriesDocument, { id: series.id, input });
		},
		onSuccess: (result) => {
			toast.success(`Renamed to ${result.updateSeriesMetadata.resolvedName}.`);
			invalidate();
			onreshaped(false);
		},
		onError: failed('Unable to rename the series.')
	}));

	const move = createMutation(() => ({
		mutationFn: () =>
			request(ConsoleMoveMediaToSeriesDocument, {
				mediaIds: selected,
				seriesId: moveTarget
			}),
		onSuccess: (result) => {
			toast.success(
				`Moved ${countNoun(selected.length, 'book')} into ${result.moveMediaToSeries.resolvedName}.`
			);
			moveTarget = '';
			invalidate();
			onreshaped(true);
		},
		onError: failed('Unable to move the books.')
	}));

	const merge = createMutation(() => ({
		mutationFn: () =>
			request(ConsoleMergeSeriesDocument, { keep: mergeTarget, drop: series.id }),
		onSuccess: (result) => {
			const { moved, missingFiles, kept } = result.mergeSeries;
			toast.success(
				`Merged into ${kept.resolvedName}: ${countNoun(moved, 'file')} moved` +
					(missingFiles ? `, ${countNoun(missingFiles, 'book')} missing from disk` : '') +
					'.'
			);
			invalidate();
			open = false;
			void goto(resolve('/(app)/series/[id]', { id: kept.id }), { replaceState: true });
		},
		onError: failed('Unable to merge the series.')
	}));

	const split = createMutation(() => ({
		mutationFn: () =>
			request(ConsoleSplitSeriesDocument, { mediaIds: selected, name: splitName.trim() }),
		onSuccess: (result) => {
			toast.success(`Split ${countNoun(selected.length, 'book')} into a new series.`);
			splitName = '';
			invalidate();
			onreshaped(true);
			open = false;
			void goto(resolve('/(app)/series/[id]', { id: result.splitSeries.id }));
		},
		onError: failed('Unable to split the books out.')
	}));

	const selectionLabel = $derived(
		selected.length === 0
			? 'Tick books in the table to move or split them.'
			: `${countNoun(selected.length, 'book')} selected.`
	);
</script>

<Sheet.Root bind:open>
	<Sheet.Content class="flex w-full flex-col gap-0 overflow-y-auto sm:max-w-md">
		<Sheet.Header>
			<Sheet.Title>Reshape {series.resolvedName}</Sheet.Title>
			<Sheet.Description>
				Moving books rewrites the files on disk into the target series' folder, so the next scan
				keeps the new grouping.
			</Sheet.Description>
		</Sheet.Header>

		<div class="flex flex-col gap-6 px-4 pb-6">
			<section class="flex flex-col gap-2">
				<h3 class="text-sm font-medium">Rename</h3>
				<Label class="text-xs text-muted-foreground" for="reshape-title">Display title</Label>
				<Input id="reshape-title" bind:value={title} />
				<p class="text-xs text-muted-foreground">
					Stored as the series' metadata title. The folder stays <code>{series.name}</code>.
				</p>
				<Button
					size="sm"
					class="w-fit"
					onclick={() => rename.mutate()}
					disabled={rename.isPending || !title.trim() || title.trim() === series.resolvedName}
				>
					{rename.isPending ? 'Renaming…' : 'Rename'}
				</Button>
			</section>

			<Separator />

			<section class="flex flex-col gap-2">
				<h3 class="text-sm font-medium">Move selected books</h3>
				<p class="text-xs text-muted-foreground">{selectionLabel}</p>
				<Select.Root
					type="single"
					value={moveTarget}
					onValueChange={(value) => (moveTarget = value)}
					disabled={siblings.length === 0}
				>
					<Select.Trigger class="w-full" aria-label="Target series">
						{siblings.length === 0 ? 'No other series in this library' : moveTargetLabel}
					</Select.Trigger>
					<Select.Content>
						{#each siblings as sibling (sibling.id)}
							<Select.Item
								value={sibling.id}
								label={`${sibling.resolvedName} (${sibling.mediaCount})`}
							/>
						{/each}
					</Select.Content>
				</Select.Root>
				<Button
					size="sm"
					class="w-fit"
					onclick={() => move.mutate()}
					disabled={move.isPending || selected.length === 0 || !moveTarget}
				>
					{move.isPending ? 'Moving…' : 'Move books'}
				</Button>
			</section>

			<Separator />

			<section class="flex flex-col gap-2">
				<h3 class="text-sm font-medium">Split selected books out</h3>
				<Label class="text-xs text-muted-foreground" for="reshape-split">New series name</Label>
				<Input id="reshape-split" bind:value={splitName} placeholder="Saga Omake" />
				<p class="text-xs text-muted-foreground">
					Creates a folder of that name in the library root and moves the selected books into it.
				</p>
				<Button
					size="sm"
					class="w-fit"
					onclick={() => split.mutate()}
					disabled={split.isPending || selected.length === 0 || !splitName.trim()}
				>
					{split.isPending ? 'Splitting…' : 'Split into new series'}
				</Button>
			</section>

			<Separator />

			<section class="flex flex-col gap-2">
				<h3 class="text-sm font-medium">Merge this series away</h3>
				<p class="text-xs text-muted-foreground">
					Every book of {series.resolvedName} moves into the series you pick; this series and its
					folder are then removed. Reading progress follows the books.
				</p>
				<Select.Root
					type="single"
					value={mergeTarget}
					onValueChange={(value) => (mergeTarget = value)}
					disabled={siblings.length === 0}
				>
					<Select.Trigger class="w-full" aria-label="Series to keep">
						{siblings.length === 0 ? 'No other series in this library' : mergeTargetLabel}
					</Select.Trigger>
					<Select.Content>
						{#each siblings as sibling (sibling.id)}
							<Select.Item
								value={sibling.id}
								label={`${sibling.resolvedName} (${sibling.mediaCount})`}
							/>
						{/each}
					</Select.Content>
				</Select.Root>
				<Button
					size="sm"
					variant="destructive"
					class="w-fit"
					onclick={() => merge.mutate()}
					disabled={merge.isPending || !mergeTarget}
				>
					{merge.isPending ? 'Merging…' : 'Merge into the selected series'}
				</Button>
			</section>
		</div>
	</Sheet.Content>
</Sheet.Root>
