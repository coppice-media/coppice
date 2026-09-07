<script lang="ts">
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import StatusBadge from '$lib/components/StatusBadge.svelte';
	import type { IngestEditionPairState, IngestItemQuery } from '$lib/graphql/generated/graphql';
	import { humanize } from '$lib/ingest/helpers';

	type Sibling = NonNullable<IngestItemQuery['ingestItem']>['dropGroupSiblings'][number];

	let {
		siblings,
		dropGroupId
	}: {
		siblings: readonly Sibling[];
		dropGroupId: string | null;
	} = $props();

	type PairBadge = { variant: 'default' | 'secondary' | 'outline'; label: string };

	// `NOT_A_PAIR` is deliberately absent: a sibling that cannot pair says
	// nothing, and an empty badge would read as a claim.
	const PAIR_BADGES: Partial<Record<IngestEditionPairState, PairBadge>> = {
		PENDING_COMMIT: { variant: 'secondary', label: 'Edition pair after commit' },
		SUGGESTED: { variant: 'default', label: 'Edition pair suggested' },
		CONFIRMED: { variant: 'default', label: 'Edition pair' },
		REJECTED: { variant: 'outline', label: 'Pair rejected' }
	};

	function pairBadge(sibling: Sibling): PairBadge | null {
		if (!sibling.editionPairCandidate) return null;
		return PAIR_BADGES[sibling.pairState] ?? null;
	}
</script>

{#if siblings.length}
	<section aria-labelledby="drop-group-heading" class="flex flex-col gap-3 rounded-lg border p-4">
		<div>
			<h2 id="drop-group-heading" class="text-lg font-semibold">From the same archive</h2>
			<p class="text-sm text-muted-foreground">
				{siblings.length} other item{siblings.length === 1 ? '' : 's'} came out of this delivery{dropGroupId ? ` · ${dropGroupId}` : ''}
			</p>
		</div>
		<div class="flex flex-col gap-2">
			{#each siblings as sibling (sibling.id)}
				{@const badge = pairBadge(sibling)}
				<div class="flex flex-wrap items-center justify-between gap-2 rounded-md border p-2">
					<div class="min-w-0">
						<p class="truncate text-sm font-medium">{sibling.filename}</p>
						<p class="text-xs text-muted-foreground">{humanize(sibling.mediaType)}</p>
					</div>
					<div class="flex flex-wrap items-center gap-2">
						{#if sibling.qualityScore !== null && sibling.qualityScore !== undefined}
							<span class="text-sm tabular-nums">{sibling.qualityScore}/100</span>
						{/if}
						<StatusBadge status={sibling.status} />
						{#if badge}
							<Badge variant={badge.variant}>{badge.label}</Badge>
						{/if}
						<Button href={`${resolve('/rework')}?item=${encodeURIComponent(sibling.id)}`} size="sm" variant="ghost">Open</Button>
					</div>
				</div>
			{/each}
		</div>
	</section>
{/if}
