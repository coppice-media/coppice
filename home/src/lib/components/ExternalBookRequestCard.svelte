<script lang="ts">
	/**
	 * One external hit on `/search`: cover, title, provider badges and the
	 * request controls. An Audible hit is an audiobook edition, so it shows
	 * a square cover, the narrator and length, and a headphones badge.
	 */
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import HeadphonesIcon from '@lucide/svelte/icons/headphones';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent } from '@stump/ui/components/ui/card';
	import ExternalHitActions from '$lib/components/requests/ExternalHitActions.svelte';
	import { safeCoverUrl } from '$lib/requests';
	import { AUDIBLE_PROVIDER, audibleHitMeta, externalHitMeta, type ExternalHit } from '$lib/search.svelte';

	let { hit }: { hit: ExternalHit } = $props();
	const audible = $derived(hit.provider === AUDIBLE_PROVIDER);
	const cover = $derived(safeCoverUrl(hit.coverUrl));
	const meta = $derived(audible ? audibleHitMeta(hit) : externalHitMeta(hit));
	const isbn = $derived(hit.isbn13 || hit.isbn10 || null);
</script>

<Card size="sm">
	<CardContent class="flex-row gap-3">
		{#if cover}
			<img
				src={cover}
				alt=""
				class={['h-18 shrink-0 rounded-md border object-cover', audible ? 'w-18' : 'w-12']}
				loading="lazy"
			/>
		{:else}
			<div
				class={['flex h-18 shrink-0 items-center justify-center rounded-md border bg-muted/40', audible ? 'w-18' : 'w-12']}
				aria-hidden="true"
			>
				<BookOpenIcon class="size-4 text-muted-foreground" />
			</div>
		{/if}
		<div class="flex min-w-0 flex-1 flex-col gap-1.5">
			<div class="min-w-0">
				<h3 class="truncate font-medium" title={hit.title}>{hit.title}</h3>
				<p class="truncate text-xs text-muted-foreground">
					{hit.authors || 'Author not provided'}{meta ? ` · ${meta}` : ''}
				</p>
			</div>
			<div class="flex flex-wrap items-center gap-1.5">
				<Badge variant="outline">{hit.provider}</Badge>
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
