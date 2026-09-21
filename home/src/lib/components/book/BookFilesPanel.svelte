<script lang="ts">
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import FileCheck2Icon from '@lucide/svelte/icons/file-check-2';
	import FileWarningIcon from '@lucide/svelte/icons/file-warning';
	import HashIcon from '@lucide/svelte/icons/hash';
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import type { BookDetail, BookEdition } from '$lib/book/detail';
	import { formatBytes, formatDate, labelKind, statusLabel } from '$lib/book/detail';

	let { detail }: { detail: BookDetail } = $props();

	function statusVariant(status: string): 'secondary' | 'destructive' | 'outline' {
		if (status === 'READY') return 'secondary';
		if (status === 'ERROR' || status === 'MISSING') return 'destructive';
		return 'outline';
	}

	function editionDescription(edition: BookEdition): string {
		if (edition.kind === 'AUDIOBOOK' && edition.audio) {
			return `${edition.audio.codec.toUpperCase()} · ${edition.audio.chapters.length} chapter${edition.audio.chapters.length === 1 ? '' : 's'}`;
		}
		if (edition.kind === 'EBOOK') return edition.file.extension.toUpperCase();
		return 'Edition';
	}
</script>

<div class="flex flex-col gap-4">
	<Card>
		<CardHeader>
			<CardTitle>Files & editions</CardTitle>
			<CardDescription>
				Confirmed editions are shown together as one work. The files and metadata remain separate records.
			</CardDescription>
		</CardHeader>
		<CardContent class="grid gap-4 lg:grid-cols-2">
			{#each detail.editions as edition (edition.mediaId)}
				<div class="flex flex-col gap-4 rounded-xl border p-4">
					<div class="flex flex-wrap items-start gap-3">
						<div class="mr-auto min-w-0">
							<div class="flex flex-wrap items-center gap-2">
								<Badge variant="secondary">{labelKind(edition.kind)}</Badge>
								<Badge variant={statusVariant(edition.file.status)}>{statusLabel(edition.file.status)}</Badge>
								{#if edition.pairEvidence}<Badge variant="outline">Confirmed · {edition.pairEvidence}</Badge>{/if}
							</div>
							<h3 class="mt-2 truncate text-sm font-semibold" title={edition.title}>{edition.title}</h3>
							<p class="text-xs text-muted-foreground">{editionDescription(edition)}</p>
						</div>
						<Button size="xs" variant="outline" href={resolve('/(app)/reader/[mediaId]', { mediaId: edition.mediaId })}>
							Open <ExternalLinkIcon data-icon="inline-end" aria-hidden="true" />
						</Button>
					</div>

					<dl class="grid gap-x-4 gap-y-2 text-xs sm:grid-cols-[7rem_1fr]">
						<dt class="text-muted-foreground">Path</dt>
						<dd class="truncate font-mono" title={edition.file.path}>{edition.file.path}</dd>
						<dt class="text-muted-foreground">Size</dt>
						<dd>{formatBytes(edition.file.size)}</dd>
						<dt class="text-muted-foreground">Modified</dt>
						<dd>{formatDate(edition.file.modifiedAt)}</dd>
						<dt class="text-muted-foreground">Extension</dt>
						<dd class="uppercase">{edition.file.extension}</dd>
						<dt class="text-muted-foreground">Stump hash</dt>
						<dd class="flex min-w-0 items-center gap-1 font-mono"><HashIcon class="size-3 shrink-0" aria-hidden="true" /><span class="truncate">{edition.file.hash ?? 'Unavailable'}</span></dd>
						<dt class="text-muted-foreground">KOReader hash</dt>
						<dd class="truncate font-mono">{edition.file.koreaderHash ?? 'Unavailable'}</dd>
					</dl>

					{#if edition.file.status === 'READY'}
						<p class="flex items-center gap-2 text-xs text-muted-foreground"><FileCheck2Icon class="size-3.5 text-emerald-500" aria-hidden="true" />Available on disk and ready to open.</p>
					{:else if edition.file.status === 'MISSING'}
						<p class="flex items-center gap-2 text-xs text-destructive"><FileWarningIcon class="size-3.5" aria-hidden="true" />Missing from the recorded path. This label comes from persisted file evidence.</p>
					{/if}
				</div>
			{:else}
				<p class="rounded-lg border border-dashed p-4 text-sm text-muted-foreground lg:col-span-2">No editions are available for this visible media.</p>
			{/each}
		</CardContent>
	</Card>
</div>
