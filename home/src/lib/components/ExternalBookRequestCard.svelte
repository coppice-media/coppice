<script lang="ts">
	/**
	 * One external hit on `/search`: cover, title, provider badges and the
	 * request controls. An Audible hit is an audiobook edition, so it shows
	 * a square cover, the narrator and length, and a headphones badge.
	 */
	import HeadphonesIcon from '@lucide/svelte/icons/headphones';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent } from '@stump/ui/components/ui/card';
	import { Cover } from '@stump/ui/components/ui/cover';
	import ExternalHitActions from '$lib/components/requests/ExternalHitActions.svelte';
	import { providerLabel, safeCoverUrl } from '$lib/requests';
	import { AUDIBLE_PROVIDER, audibleHitMeta, externalHitMeta, type ExternalHit } from '$lib/search.svelte';

	let { hit }: { hit: ExternalHit } = $props();
	const audible = $derived(hit.provider === AUDIBLE_PROVIDER);
	const cover = $derived(safeCoverUrl(hit.coverUrl));
	const meta = $derived(audible ? audibleHitMeta(hit) : externalHitMeta(hit));
	const isbn = $derived(hit.isbn13 || hit.isbn10 || null);
</script>

<Card size="sm">
	<CardContent class="flex-row gap-3">
		<Cover
			src={cover}
			aspect={audible ? 'square' : 'book'}
			class={['h-18 rounded-md border', audible ? 'w-18' : 'w-12']}
		/>
		<div class="flex min-w-0 flex-1 flex-col gap-1.5">
			<div class="min-w-0">
				<h3 class="truncate font-medium" title={hit.title}>{hit.title}</h3>
				<p class="truncate text-xs text-muted-foreground">
					{hit.authors || 'Author not provided'}{meta ? ` · ${meta}` : ''}
				</p>
			</div>
			<div class="flex flex-wrap items-center gap-1.5">
				<Badge variant="outline">{providerLabel(hit.provider)}</Badge>
				{#if audible}
					<Badge variant="outline">
						<HeadphonesIcon aria-hidden="true" />
						Audiobook
					</Badge>
				{/if}
				{#if isbn}
					<Badge variant="outline" class="font-mono">{isbn}</Badge>
				{/if}
			</div>
			<div class="mt-auto flex justify-end">
				<ExternalHitActions {hit} />
			</div>
		</div>
	</CardContent>
</Card>
