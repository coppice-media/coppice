<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import { request } from '@stump/ui/graphql/client';
	import { ConsoleEntityBookCountDocument } from '$lib/graphql/generated/graphql';
	import { countNoun } from '$lib/format';
	import { offsetPage } from '$lib/library';
	import type { EntityTileData } from './entities';

	let { entity }: { entity: EntityTileData } = $props();

	// Only tiles without a count of their own ask the server, one small query
	// per visible tile — the page size bounds it.
	const countQuery = createQuery(() => ({
		queryKey: ['entityBookCount', entity.key],
		queryFn: () => request(ConsoleEntityBookCountDocument, { filter: entity.filter ?? {} }),
		enabled: browser && entity.bookCount === undefined && !!entity.filter
	}));
	const count = $derived(
		entity.bookCount ?? offsetPage(countQuery.data?.media.pageInfo)?.totalItems ?? null
	);
</script>

<a
	class="flex items-center gap-3 rounded-xl border bg-card px-4 py-3 transition-colors hover:bg-muted"
	href={entity.href}
>
	<div class="min-w-0 flex-1">
		<div class="truncate font-medium">{entity.name}</div>
		<div class="text-xs text-muted-foreground">
			{#if count === null}
				Counting…
			{:else}
				{countNoun(count, 'book')}
			{/if}
			{#if entity.detail}
				· {entity.detail}
			{/if}
		</div>
	</div>
	<ChevronRightIcon class="size-4 shrink-0 text-muted-foreground" />
</a>
