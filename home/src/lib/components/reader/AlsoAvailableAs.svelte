<script lang="ts">
	/**
	 * "Also available as" — the other editions of the book being read.
	 *
	 * Confirmed editions get an **Open here** action that opens the other
	 * reader at the *converted* position: an audio offset becomes a locator
	 * inside the mapped spine item, and a locator becomes a millisecond
	 * offset inside the mapped chapter. The conversion goes through the pair's
	 * chapter map, so it is worth ±1-3 minutes on a 30-minute chapter — hence
	 * the **approximate** badge, and hence the jump is a deep link that the
	 * target reader does not write back until the reader actually pages.
	 *
	 * Suggestions are the pairing heuristics' guesses. They are shown as
	 * guesses, with the evidence that produced them, and nothing converts
	 * through one: confirming is a decision a person makes.
	 */
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { request } from '@stump/ui/graphql/client';
	import {
		ReaderConfirmEditionPairDocument,
		ReaderEditionsDocument,
		ReaderRejectEditionPairDocument,
		type PairEvidence
	} from '$lib/graphql/generated/graphql';

	let { mediaId }: { mediaId: string } = $props();

	const client = useQueryClient();

	const editionsQuery = createQuery(() => ({
		queryKey: ['readerEditions', mediaId],
		queryFn: () => request(ReaderEditionsDocument, { id: mediaId }),
		enabled: browser && mediaId.length > 0
	}));
	const editions = $derived(editionsQuery.data?.mediaById?.editions ?? []);
	const suggestions = $derived(editionsQuery.data?.mediaById?.editionSuggestions ?? []);

	/** Why pairing believes a suggestion, in words an operator can act on. */
	const EVIDENCE_LABELS: Record<PairEvidence, string> = {
		MANUAL: 'paired by hand',
		WORK_ID: 'same work',
		PROVIDER_EDITION_LIST: 'identifier matched through a provider edition list',
		SAME_DROP: 'arrived in one drop',
		TITLE_AUTHOR: 'title and author match'
	};

	function invalidate(): void {
		void client.invalidateQueries({ queryKey: ['readerEditions'] });
	}

	const confirmPair = createMutation(() => ({
		mutationFn: (otherId: string) =>
			request(ReaderConfirmEditionPairDocument, { mediaIdA: mediaId, mediaIdB: otherId }),
		onSuccess: ({ confirmEditionPair: result }) => {
			if (result.changed) {
				toast.success('Paired.', {
					description: result.chapterMapEntries
						? `Mapped ${result.chapterMapEntries} chapters between the two editions.`
						: 'No chapter map: these two editions have no time to convert.'
				});
			} else {
				toast.info(result.message ?? 'Already paired.');
			}
			invalidate();
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to pair the editions.')
	}));

	const rejectPair = createMutation(() => ({
		mutationFn: (otherId: string) =>
			request(ReaderRejectEditionPairDocument, { mediaIdA: mediaId, mediaIdB: otherId }),
		onSuccess: ({ rejectEditionPair: result }) => {
			toast.success(result.changed ? 'Suggestion dismissed.' : (result.message ?? 'Nothing to dismiss.'));
			invalidate();
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to dismiss the suggestion.')
	}));

	type Edition = (typeof editions)[number];

	/**
	 * Where "Open here" goes. The converted position travels as the same deep
	 * link the annotation hub uses — `href` + `itemProgression` for an EPUB,
	 * `positionMs` for a recording — so the target reader opens there without
	 * the jump being saved as a real position.
	 */
	function openHereHref(edition: Edition): string {
		const base = resolve('/(app)/reader/[mediaId]', { mediaId: edition.id });
		const mapped = edition.pairedPosition;
		if (!mapped) return base;

		const params = new URLSearchParams();
		if (mapped.positionMs != null) {
			params.set('positionMs', String(Math.round(mapped.positionMs)));
		}
		if (mapped.locator?.href) {
			params.set('href', mapped.locator.href);
			const within = mapped.locator.locations?.progression;
			if (within != null) params.set('itemProgression', String(within));
			params.set('progression', String(mapped.progression));
		}
		const query = params.toString();
		return query ? `${base}?${query}` : base;
	}

	function formatLabel(edition: { extension: string; audio?: unknown }): string {
		return edition.audio ? 'Audiobook' : edition.extension.toUpperCase();
	}

	function positionLabel(edition: Edition): string | null {
		const mapped = edition.pairedPosition;
		if (!mapped) return null;
		if (mapped.positionMs != null) {
			const total = Math.round(mapped.positionMs / 1000);
			const minutes = Math.floor(total / 60);
			const seconds = total % 60;
			return `~${minutes}:${String(seconds).padStart(2, '0')}`;
		}
		return `~${Math.round(mapped.progression * 100)}%`;
	}
</script>

{#if editions.length > 0 || suggestions.length > 0}
	<section
		class="flex flex-col gap-2 rounded-xl border bg-card p-3"
		aria-label="Also available as"
	>
		<h2 class="text-sm font-semibold tracking-tight">Also available as</h2>

		{#each editions as edition (edition.id)}
			<div class="flex flex-wrap items-center gap-2 text-sm">
				<Badge variant="secondary">{formatLabel(edition)}</Badge>
				<span class="mr-auto">{edition.resolvedName}</span>
				{#if edition.pairedPosition}
					<Badge
						variant="outline"
						title="Converted through the pair's chapter map: good enough to resume near, not to highlight"
					>
						approximate
					</Badge>
					<span class="text-xs text-muted-foreground">{positionLabel(edition)}</span>
				{/if}
				<Button size="xs" variant="secondary" href={openHereHref(edition)}>
					{edition.pairedPosition ? 'Open here' : 'Open'}
				</Button>
			</div>
		{/each}

		{#each suggestions as suggestion (suggestion.media.id)}
			<div class="flex flex-wrap items-center gap-2 text-sm">
				<Badge variant="outline">{formatLabel(suggestion.media)}</Badge>
				<span class="mr-auto">
					{suggestion.media.resolvedName}
					<span class="text-xs text-muted-foreground">
						· suggested ({EVIDENCE_LABELS[suggestion.evidence ?? 'TITLE_AUTHOR']})
					</span>
				</span>
				<Button
					size="xs"
					onclick={() => confirmPair.mutate(suggestion.media.id)}
					disabled={confirmPair.isPending}
				>
					{confirmPair.isPending ? 'Pairing…' : 'Same book'}
				</Button>
				<Button
					size="xs"
					variant="ghost"
					onclick={() => rejectPair.mutate(suggestion.media.id)}
					disabled={rejectPair.isPending}
				>
					Not the same
				</Button>
			</div>
		{/each}
	</section>
{/if}
