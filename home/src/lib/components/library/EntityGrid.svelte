<script lang="ts">
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import EntityTile from './EntityTile.svelte';
	import type { EntityTileData } from './entities';

	let {
		entities,
		emptyTitle,
		emptyDescription
	}: {
		entities: EntityTileData[];
		emptyTitle: string;
		emptyDescription: string;
	} = $props();
</script>

{#if entities.length === 0}
	<Empty class="rounded-xl border border-dashed bg-card">
		<EmptyHeader>
			<EmptyTitle>{emptyTitle}</EmptyTitle>
			<EmptyDescription>{emptyDescription}</EmptyDescription>
		</EmptyHeader>
	</Empty>
{:else}
	<ul class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
		{#each entities as entity (entity.key)}
			<li>
				<EntityTile {entity} />
			</li>
		{/each}
	</ul>
{/if}
