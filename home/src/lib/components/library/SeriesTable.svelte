<script lang="ts">
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import * as Table from '@stump/ui/components/ui/table';
	import { countLabel } from '$lib/format';
	import type { ConsoleSeriesCardFragment } from '$lib/graphql/generated/graphql';

	let { series }: { series: ConsoleSeriesCardFragment[] } = $props();
</script>

<div class="overflow-x-auto rounded-xl border bg-card">
	<Table.Root>
		<Table.Header>
			<Table.Row>
				<Table.Head>Series</Table.Head>
				<Table.Head class="text-right">Books</Table.Head>
				<Table.Head class="text-right">Unread</Table.Head>
				<Table.Head class="text-right">Progress</Table.Head>
				<Table.Head>Publisher</Table.Head>
				<Table.Head>Status</Table.Head>
			</Table.Row>
		</Table.Header>
		<Table.Body>
			{#each series as entry (entry.id)}
				<Table.Row>
					<Table.Cell>
						<a class="font-medium hover:underline" href={resolve('/(app)/series/[id]', { id: entry.id })}>
							{entry.resolvedName}
						</a>
						<div class="truncate text-xs text-muted-foreground" title={entry.path}>
							{entry.path}
						</div>
					</Table.Cell>
					<Table.Cell class="text-right tabular-nums">{countLabel(entry.mediaCount)}</Table.Cell>
					<Table.Cell class="text-right tabular-nums">{countLabel(entry.unreadCount)}</Table.Cell>
					<Table.Cell class="text-right tabular-nums">
						{Math.round(entry.percentageCompleted)}%
					</Table.Cell>
					<Table.Cell class="text-muted-foreground">
						{entry.metadata?.publisher ?? '—'}
					</Table.Cell>
					<Table.Cell>
						{#if entry.status === 'READY'}
							<span class="text-muted-foreground">Ready</span>
						{:else}
							<Badge variant="destructive">{entry.status}</Badge>
						{/if}
					</Table.Cell>
				</Table.Row>
			{/each}
		</Table.Body>
	</Table.Root>
</div>
