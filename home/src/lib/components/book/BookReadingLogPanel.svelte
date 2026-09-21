<script lang="ts">
	import BookOpenCheckIcon from '@lucide/svelte/icons/book-open-check';
	import Clock3Icon from '@lucide/svelte/icons/clock-3';
	import SmartphoneIcon from '@lucide/svelte/icons/smartphone';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import type { BookReadingLog, BookReadingSession } from '$lib/book/detail';
	import { formatDate, formatDuration, labelKind } from '$lib/book/detail';

	let { log }: { log?: BookReadingLog | null } = $props();

	function sessionLabel(session: BookReadingSession): string {
		if (session.status === 'FINISHED' || session.status === 'COMPLETED') return 'Completed';
		if (session.status === 'ACTIVE' || session.status === 'IN_PROGRESS') return 'In progress';
		return session.status.replaceAll('_', ' ').toLowerCase();
	}
</script>

<Card>
	<CardHeader>
		<CardTitle class="flex items-center gap-2"><BookOpenCheckIcon class="size-4" aria-hidden="true" />Reading log</CardTitle>
		<CardDescription>Progress and sessions from the linked editions. Device and protocol attribution is preserved.</CardDescription>
	</CardHeader>
	<CardContent>
		{#if !log || log.editions.length === 0}
			<p class="rounded-lg border border-dashed p-5 text-sm text-muted-foreground">No reading history has been recorded for this work.</p>
		{:else}
			<div class="grid gap-5">
				{#each log.editions as edition (edition.mediaId)}
					<section class="rounded-xl border p-4">
						<div class="flex flex-wrap items-center gap-2">
							<h3 class="mr-auto text-sm font-semibold">{labelKind(edition.kind)}</h3>
							<Badge variant="outline">{edition.sessions.length} session{edition.sessions.length === 1 ? '' : 's'}</Badge>
						</div>
						{#if edition.head}
							<div class="mt-3 grid gap-2 rounded-lg bg-muted/40 p-3 text-xs sm:grid-cols-2">
								<div><span class="text-muted-foreground">Current position</span><p class="font-medium">{edition.head.page == null ? edition.head.progression == null ? 'Unavailable' : `${Math.round(edition.head.progression * 100)}%` : `Page ${edition.head.page}`}</p></div>
								<div><span class="text-muted-foreground">Last updated</span><p class="font-medium">{formatDate(edition.head.updatedAt)}</p></div>
								<div><span class="text-muted-foreground">Protocol</span><p class="font-medium">{edition.head.sourceProtocol ?? 'Unavailable'}</p></div>
								<div><span class="text-muted-foreground">Device</span><p class="flex items-center gap-1 font-medium"><SmartphoneIcon class="size-3" aria-hidden="true" />{edition.head.sourceDevice?.name ?? edition.head.sourceDeviceId ?? 'Unavailable'}</p></div>
							</div>
						{:else}
							<p class="mt-3 text-xs text-muted-foreground">No current head position.</p>
						{/if}
						{#if edition.sessions.length}
							<div class="mt-4 grid gap-2">
								{#each edition.sessions as session (session.id)}
									<div class="flex flex-wrap items-start gap-3 border-t pt-3 text-xs">
										<div class="mr-auto min-w-0">
											<div class="flex flex-wrap items-center gap-2"><Badge variant="outline">{sessionLabel(session)}</Badge><span class="text-muted-foreground">Readthrough {session.readthroughNumber}</span></div>
											<p class="mt-1 text-muted-foreground">{formatDate(session.sessionDate ?? session.createdAt)}{session.updatedAt ? ` → ${formatDate(session.updatedAt)}` : ''}</p>
											{#if session.sourceProtocol}<p class="mt-1">Source: {session.sourceProtocol}</p>{/if}
											{#if session.sourceDevices.length}<p class="mt-1 flex flex-wrap gap-2">{#each session.sourceDevices as device}<span class="inline-flex items-center gap-1 text-muted-foreground"><SmartphoneIcon class="size-3" aria-hidden="true" />{device.name ?? device.id}</span>{/each}</p>{:else if session.sourceDeviceIds.length}<p class="mt-1 text-muted-foreground">Devices: {session.sourceDeviceIds.join(', ')}</p>{/if}
										</div>
										<div class="flex items-center gap-1 text-muted-foreground"><Clock3Icon class="size-3" aria-hidden="true" />{session.elapsedSeconds == null ? 'Unavailable' : formatDuration(session.elapsedSeconds * 1000)}</div>
									</div>
								{/each}
							</div>
						{/if}
					</section>
				{/each}
			</div>
		{/if}
	</CardContent>
</Card>
