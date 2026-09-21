<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import FileCheck2Icon from '@lucide/svelte/icons/file-check-2';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import * as AlertDialog from '@stump/ui/components/ui/alert-dialog';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { safeFilePreview, formatBytes, releaseReasons, releaseScore, sortReleases, type ReleaseLike } from '$lib/requests';

	type Release = ReleaseLike & {
		id: string;
		title: string;
		sourceProvider: string;
		remoteId: string;
		authors?: string | null;
		previewName?: string | null;
		previewMime?: string | null;
		previewBytes?: number | null;
	};

	let {
		releases,
		selectedReleaseId = null,
		minimumScore = 0,
		busy = false,
		disabled = false,
		onconfirm
	}: {
		releases: readonly Release[];
		selectedReleaseId?: string | null;
		minimumScore?: number;
		busy?: boolean;
		disabled?: boolean;
		onconfirm?: (release: Release) => void;
	} = $props();

	let pending = $state<Release | null>(null);
	const sorted = $derived(sortReleases(releases));


	function askToSelect(release: Release): void {
		if (!disabled && !busy) pending = release;
	}

	function confirmSelection(): void {
		if (!pending || disabled || busy) return;
		onconfirm?.(pending);
		pending = null;
	}
</script>

<Card aria-labelledby="release-picker-heading">
	<CardHeader>
		<div class="flex flex-wrap items-start gap-3">
			<div class="mr-auto">
				<CardTitle id="release-picker-heading" class="text-base">Choose a release</CardTitle>
				<p class="mt-1 text-sm text-muted-foreground">
					Rows are ranked by the server score. Releases below the {minimumScore} / 100 floor cannot be selected. We never show tracker URLs or download credentials.
				</p>
			</div>
			<Badge variant="outline">{sorted.length} result{sorted.length === 1 ? '' : 's'}</Badge>
		</div>
	</CardHeader>
	<CardContent class="flex flex-col gap-3">
		{#if sorted.length === 0}
			<Alert>
				<AlertTitle>No releases yet</AlertTitle>
				<AlertDescription>Run source search again after checking the gateway status.</AlertDescription>
			</Alert>
		{:else}
			<div class="flex flex-col gap-3" role="list" aria-label="Available releases">
				{#each sorted as release (release.id)}
					{@const score = releaseScore(release)}
					{@const reasons = releaseReasons(release)}
					{@const selectable = score >= minimumScore}
					<div
						role="listitem"
						class="rounded-lg border p-3 transition-colors {selectedReleaseId === release.id ? 'border-primary bg-primary/5' : 'bg-background/40'} {!selectable ? 'opacity-70' : ''}"
					>
						<div class="flex flex-wrap items-start gap-3">
							<div class="min-w-0 flex-1">
								<div class="flex flex-wrap items-center gap-2">
									<h3 class="truncate font-medium">{release.title}</h3>
									<Badge variant={selectedReleaseId === release.id ? 'default' : 'secondary'}>
										Score {score}
									</Badge>
									{#if !selectable}
										<Badge variant="destructive">Below floor</Badge>
									{/if}
									{#if selectedReleaseId === release.id}
										<Badge variant="outline">Selected</Badge>
									{/if}
								</div>
								<p class="mt-1 text-sm text-muted-foreground">
									{release.sourceProvider}{release.authors ? ` · ${release.authors}` : ''}
								</p>
								<ul class="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground" aria-label="Why this release scored well">
									{#each reasons as reason (reason)}
										<li>{reason}</li>
									{/each}
								</ul>
							</div>
							<Button
								variant={selectedReleaseId === release.id ? 'secondary' : 'outline'}
								size="sm"
								disabled={disabled || busy || selectedReleaseId === release.id || !selectable}
								onclick={() => askToSelect(release)}
							>
								{#if selectedReleaseId === release.id}
									<CheckIcon data-icon="inline-start" aria-hidden="true" />
									Selected
								{:else if !selectable}
									Below floor
								{:else}
									<FileCheck2Icon data-icon="inline-start" aria-hidden="true" />
									Select
								{/if}
							</Button>
						</div>

						<dl class="mt-3 grid grid-cols-2 gap-x-4 gap-y-2 border-t pt-3 text-xs sm:grid-cols-4">
							<div>
								<dt class="text-muted-foreground">Format</dt>
								<dd class="mt-0.5 font-medium">{release.format?.toUpperCase() || 'Unknown'}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Quality</dt>
								<dd class="mt-0.5 font-medium">{release.quality || 'Not provided'}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Size</dt>
								<dd class="mt-0.5 font-medium">{formatBytes(release.sizeBytes ?? release.previewBytes)}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Seeders</dt>
								<dd class="mt-0.5 font-medium">{release.seeders ?? 'Unknown'}</dd>
							</div>
						</dl>

						<div class="mt-3 flex min-w-0 items-start gap-2 rounded-md bg-muted/40 px-2.5 py-2 text-xs text-muted-foreground">
							<ShieldCheckIcon class="mt-0.5 size-3.5 shrink-0 text-primary" aria-hidden="true" />
							<span class="min-w-0 break-words">
								Safe preview: <span class="font-mono">{safeFilePreview(release.previewName)}</span>
								{#if release.previewMime} · {release.previewMime}{/if}
								{#if release.previewBytes} · {formatBytes(release.previewBytes)}{/if}
							</span>
						</div>
					</div>
				{/each}
			</div>
		{/if}
	</CardContent>
</Card>

<AlertDialog.Root
	open={pending !== null}
	onOpenChange={(open) => {
		if (!open && !busy) pending = null;
	}}
>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Use this release?</AlertDialog.Title>
			<AlertDialog.Description>
				{#if pending}
					This stores the selected opaque release id and starts no download yet. You will confirm the grab separately.
					<strong class="mt-2 block text-foreground">{pending.title}</strong>
					<span class="block">{pending.sourceProvider} · score {releaseScore(pending)}</span>
				{:else}
					Select a release to continue.
				{/if}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={busy}>Keep browsing</AlertDialog.Cancel>
			<AlertDialog.Action onclick={confirmSelection} disabled={!pending || busy}>
				{busy ? 'Saving…' : 'Confirm selection'}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
