<script lang="ts">
	import { resolve } from '$app/paths';
	import { createMutation } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Checkbox } from '@stump/ui/components/ui/checkbox';
	import { Progress } from '@stump/ui/components/ui/progress';
	import * as Table from '@stump/ui/components/ui/table';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleFinishMediaDocument,
		ConsoleResetMediaProgressDocument,
		type ConsoleBookRowFragment
	} from '$lib/graphql/generated/graphql';
	import { bytesLabel, countLabel, relativeTime } from '$lib/format';
	import { READING_STATUS_LABELS, bookProgress, progressPercent } from '$lib/library';
	import SendToKindleButton from './SendToKindleButton.svelte';

	let {
		books,
		selected = $bindable([]),
		selectable = false,
		showSeries = false,
		onchange
	}: {
		books: ConsoleBookRowFragment[];
		selected?: string[];
		selectable?: boolean;
		showSeries?: boolean;
		onchange?: () => void;
	} = $props();

	const allSelected = $derived(books.length > 0 && books.every((book) => selected.includes(book.id)));

	const finish = createMutation(() => ({
		mutationFn: (id: string) => request(ConsoleFinishMediaDocument, { id }),
		onSuccess: () => {
			toast.success('Marked as read.');
			onchange?.();
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to mark the book as read.')
	}));
	const reset = createMutation(() => ({
		mutationFn: (id: string) => request(ConsoleResetMediaProgressDocument, { id }),
		onSuccess: () => {
			toast.success('Progress and history cleared.');
			onchange?.();
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to clear the progress.')
	}));

	function toggle(id: string, checked: boolean): void {
		selected = checked ? [...selected, id] : selected.filter((entry) => entry !== id);
	}

	function toggleAll(checked: boolean): void {
		selected = checked ? books.map((book) => book.id) : [];
	}
</script>

<div class="overflow-x-auto rounded-xl border bg-card">
	<Table.Root>
		<Table.Header>
			<Table.Row>
				{#if selectable}
					<Table.Head class="w-10">
						<Checkbox
							checked={allSelected}
							indeterminate={!allSelected && selected.length > 0}
							onCheckedChange={toggleAll}
							aria-label="Select every book on this page"
						/>
					</Table.Head>
				{/if}
				<Table.Head>Book</Table.Head>
				{#if showSeries}
					<Table.Head>Series</Table.Head>
				{/if}
				<Table.Head>Format</Table.Head>
				<Table.Head class="text-right">Pages</Table.Head>
				<Table.Head class="w-48">Progress</Table.Head>
				<Table.Head class="text-right">Actions</Table.Head>
			</Table.Row>
		</Table.Header>
		<Table.Body>
			{#each books as book (book.id)}
				{@const progress = bookProgress(book)}
				<Table.Row>
					{#if selectable}
						<Table.Cell>
							<Checkbox
								checked={selected.includes(book.id)}
								onCheckedChange={(checked) => toggle(book.id, checked)}
								aria-label={`Select ${book.resolvedName}`}
							/>
						</Table.Cell>
					{/if}
					<Table.Cell>
						<a
							class="font-medium hover:underline"
							href={resolve('/(app)/reader/[mediaId]', { mediaId: book.id })}
						>
							{book.resolvedName}
						</a>
						<div class="text-xs text-muted-foreground">
							{bytesLabel(book.size)}
							{#if book.status !== 'READY'}
								· <Badge variant="destructive">{book.status}</Badge>
							{/if}
						</div>
					</Table.Cell>
					{#if showSeries}
						<Table.Cell class="text-muted-foreground">
							<a
								class="hover:underline"
								href={resolve('/(app)/series/[id]', { id: book.series.id })}
							>
								{book.series.resolvedName}
							</a>
						</Table.Cell>
					{/if}
					<Table.Cell class="uppercase text-muted-foreground">{book.extension}</Table.Cell>
					<Table.Cell class="text-right tabular-nums">
						{book.pages > 0 ? countLabel(book.pages) : '—'}
					</Table.Cell>
					<Table.Cell>
						<div class="flex flex-col gap-1">
							<span class="text-xs text-muted-foreground">
								{READING_STATUS_LABELS[progress.status]}
								{#if progress.status === 'READING'}
									· {progressPercent(progress)}%
									{#if book.readProgress?.updatedAt}
										· {relativeTime(book.readProgress.updatedAt)}
									{/if}
								{/if}
							</span>
							<Progress value={progressPercent(progress)} />
						</div>
					</Table.Cell>
					<Table.Cell class="text-right">
						<div class="flex justify-end gap-1">
							<Button
								size="xs"
								variant="outline"
								href={resolve('/(app)/reader/[mediaId]', { mediaId: book.id })}
							>
								Read
							</Button>
							<SendToKindleButton mediaId={book.id} />
							{#if progress.status === 'FINISHED'}
								<Button
									size="xs"
									variant="ghost"
									onclick={() => reset.mutate(book.id)}
									disabled={reset.isPending}
								>
									Mark unread
								</Button>
							{:else}
								<Button
									size="xs"
									variant="ghost"
									onclick={() => finish.mutate(book.id)}
									disabled={finish.isPending}
								>
									Mark read
								</Button>
								{#if progress.status !== 'NOT_STARTED'}
									<Button
										size="xs"
										variant="ghost"
										onclick={() => reset.mutate(book.id)}
										disabled={reset.isPending}
									>
										Reset
									</Button>
								{/if}
							{/if}
						</div>
					</Table.Cell>
				</Table.Row>
			{/each}
		</Table.Body>
	</Table.Root>
</div>
