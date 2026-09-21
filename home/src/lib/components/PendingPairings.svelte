<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import ScanLineIcon from '@lucide/svelte/icons/scan-line';
	import { REGEXP_ONLY_DIGITS } from 'bits-ui';
	import { toast } from 'svelte-sonner';
	import * as AlertDialog from '@stump/ui/components/ui/alert-dialog';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import * as InputOTP from '@stump/ui/components/ui/input-otp';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import { errorMessage } from '@stump/ui/utils/errors.js';
	import {
		ApproveDevicePairingDocument,
		DenyDevicePairingDocument,
		PendingDevicePairingsDocument
	} from '$lib/graphql/generated/graphql';
	import { DEVICE_KIND_ICONS, DEVICE_KIND_LABELS } from '$lib/devices';
	import { relativeTime } from '$lib/format';
	let { now }: { now: Date } = $props();

	const CODE_LENGTH = 6;

	const queryClient = useQueryClient();
	const pairings = createQuery(() => ({
		queryKey: ['pendingDevicePairings'],
		queryFn: () => request(PendingDevicePairingsDocument, {}),
		enabled: browser,
		// Pairings expire in minutes; keep the list honest without a subscription.
		refetchInterval: 10_000
	}));
	const pending = $derived(
		(pairings.data?.pendingDevicePairings ?? []).filter((pairing) => pairing.status === 'PENDING')
	);

	let codes = $state<Record<string, string>>({});
	let denyConfirm = $state<string | null>(null);
	function settle(): void {
		void queryClient.invalidateQueries({ queryKey: ['pendingDevicePairings'] });
		void queryClient.invalidateQueries({ queryKey: ['devices'] });
	}

	const approve = createMutation(() => ({
		mutationFn: (pairingId: string) =>
			request(ApproveDevicePairingDocument, { pairingId, code: codes[pairingId] ?? '' }),
		onSuccess: () => {
			toast.success('Pairing approved; the device picks up its credential on its next poll.');
			settle();
		},
		onError: (error, pairingId) => {
			// A wrong code counts against the pairing's five attempts, so the
			// boxes clear for a fresh try instead of resubmitting the same digits.
			codes[pairingId] = '';
			toast.error(errorMessage(error));
		}
	}));
	const deny = createMutation(() => ({
		mutationFn: (pairingId: string) => request(DenyDevicePairingDocument, { pairingId }),
		onSuccess: () => {
			denyConfirm = null;
			toast.success('Pairing denied.');
			settle();
		},
		onError: (error) => toast.error(errorMessage(error))
	}));

	/** Submits a complete code once; a filled last box and the Approve button both land here. */
	function submitCode(pairingId: string): void {
		if (approve.isPending || (codes[pairingId] ?? '').length !== CODE_LENGTH) return;
		approve.mutate(pairingId);
	}
</script>

<section class="flex flex-col gap-3" aria-labelledby="pending-pairings-heading">
	<h2 id="pending-pairings-heading" class="sr-only">Pairing requests</h2>
	{#if pairings.isPending}
		<div class="flex flex-col gap-3" aria-label="Loading pairing requests">
			<Skeleton class="h-24 rounded-xl" />
		</div>
	{:else if pairings.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load pairing requests</AlertTitle>
			<AlertDescription>{errorMessage(pairings.error)}</AlertDescription>
			<Button type="button" variant="outline" class="mt-3" onclick={() => pairings.refetch()}>
				Retry
			</Button>
		</Alert>
	{:else if pending.length === 0}
		<p class="flex items-start gap-2 text-xs text-muted-foreground">
			<ScanLineIcon class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
			<span>
				Pairing starts on the device: in NickelCoppice, coppice.koplugin, CrossPoint, or the Mihon extension, choose Coppice and it shows a six-digit code. New requests appear here within ten seconds.
			</span>
		</p>
	{:else}
		{#each pending as pairing (pairing.id)}
			{@const Icon = DEVICE_KIND_ICONS[pairing.kind]}
			<div
				class="flex flex-col gap-4 rounded-xl border bg-card p-4 sm:p-5 lg:flex-row lg:items-center"
			>
				<div class="flex min-w-0 flex-1 items-start gap-3">
					<span
						class="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary"
					>
						<Icon class="size-5" aria-hidden="true" />
					</span>
					<div class="min-w-0 flex-1">
						<p class="text-sm font-medium">A device is asking to connect</p>
						<p class="text-sm text-muted-foreground">
							{DEVICE_KIND_LABELS[pairing.kind]} · {pairing.name ?? 'Unnamed device'} · from
							{pairing.remoteIp}
						</p>
						<p class="mt-0.5 text-xs text-muted-foreground">
							Requested {relativeTime(pairing.createdAt, now)} · expires
							{relativeTime(pairing.expiresAt, now)}
							{#if pairing.failedAttempts}
								· {pairing.failedAttempts} wrong {pairing.failedAttempts === 1 ? 'code' : 'codes'}
								so far
							{/if}
						</p>
						{#if String(pairing.kind) === 'CROSSPOINT'}
							<p class="mt-1 text-xs text-muted-foreground">
								CrossPoint receives its one-time keyed KOReader-compatible sync URL after approval. Home never asks for a Coppice password; keep File Transfer / Calibre Wireless active for LAN delivery.
							</p>
						{/if}
					</div>
				</div>
				<form
					class="flex flex-wrap items-center gap-3"
					onsubmit={(event) => {
						event.preventDefault();
						submitCode(pairing.id);
					}}
				>
					<InputOTP.Root
						maxlength={CODE_LENGTH}
						pattern={REGEXP_ONLY_DIGITS}
						bind:value={codes[pairing.id]}
						aria-label={`Pairing code for ${pairing.name ?? 'unnamed device'}`}
						onComplete={() => submitCode(pairing.id)}
						disabled={approve.isPending || deny.isPending}
					>
						{#snippet children({ cells })}
							<InputOTP.Group>
								{#each cells as cell, index (index)}
									<InputOTP.Slot {cell} class="size-11 text-lg font-medium tabular-nums" />
								{/each}
							</InputOTP.Group>
						{/snippet}
					</InputOTP.Root>
					<div class="flex items-center gap-2">
						<Button
							type="submit"
							disabled={
								approve.isPending ||
								deny.isPending ||
								(codes[pairing.id] ?? '').length !== CODE_LENGTH
							}
						>
							{approve.isPending ? 'Approving…' : 'Approve'}
						</Button>
						<Button
							type="button"
							variant="ghost"
							onclick={() => (denyConfirm = pairing.id)}
							disabled={deny.isPending || approve.isPending}
						>
							Deny
						</Button>
					</div>
				</form>
			</div>
		{/each}
	{/if}
</section>

<AlertDialog.Root
	open={denyConfirm !== null}
	onOpenChange={(value) => {
		if (!value && !deny.isPending) denyConfirm = null;
	}}
>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Deny this pairing request?</AlertDialog.Title>
			<AlertDialog.Description>
				The request for {pending.find((pairing) => pairing.id === denyConfirm)?.name ??
					'an unnamed device'} will be discarded. The device must start pairing again.
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={deny.isPending}>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action onclick={() => denyConfirm && deny.mutate(denyConfirm)} disabled={deny.isPending}>
				{deny.isPending ? 'Denying…' : 'Deny pairing'}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
