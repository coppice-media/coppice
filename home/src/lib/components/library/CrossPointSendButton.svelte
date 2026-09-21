<script lang="ts">
	import { browser } from '$app/environment'
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query'
	import SendIcon from '@lucide/svelte/icons/send'
	import { toast } from 'svelte-sonner'
	import { Button } from '@stump/ui/components/ui/button'
	import * as Select from '@stump/ui/components/ui/select'
	import { request } from '@stump/ui/graphql/client'
	import { CrosspointTargetDocument, DevicesDocument, QueueCrosspointDeliveriesDocument } from '$lib/graphql/generated/graphql'
	import { errorMessage } from '@stump/ui/utils/errors.js'

	let {
		mediaId = null,
		mediaIds = [],
		deviceId = null,
		targetPath = ''
	}: {
		/** The single visible media id for a row/detail action. */
		mediaId?: string | null
		/** A selected visible set for the bulk-library action. */
		mediaIds?: readonly string[]
		/** Set this for a dedicated-device action; otherwise the user picks a target. */
		deviceId?: string | null
		/** Optional explicit path; otherwise the verified target's saved root is used. */
		targetPath?: string
	} = $props()

	const queryClient = useQueryClient()
	const devicesQuery = createQuery(() => ({
		queryKey: ['devices'],
		queryFn: () => request(DevicesDocument, {}),
		enabled: browser
	}))
	const targets = $derived(
		(devicesQuery.data?.devices ?? []).filter(
			(device) => String(device.kind) === 'CROSSPOINT' && !device.revokedAt
		)
	)
	const ids = $derived(
		[...new Set(mediaIds.length ? mediaIds : mediaId ? [mediaId] : [])].filter(Boolean)
	)
	const count = $derived(ids.length)
	const dedicatedTarget = $derived(
		deviceId ? targets.find((target) => target.id === deviceId) ?? null : null
	)
	const selectableTargets = $derived(deviceId ? (dedicatedTarget ? [dedicatedTarget] : []) : targets)

	const queue = createMutation(() => ({
		mutationFn: async (targetId: string) => {
			let destination = targetPath.trim()
			if (!destination) {
				const result = await request(CrosspointTargetDocument, { deviceId: targetId })
				destination =
					(result as { crosspointTarget?: { rootPath?: string | null } | null }).crosspointTarget?.rootPath ??
					'/'
			}
			return request(QueueCrosspointDeliveriesDocument, {
				deviceId: targetId,
				mediaIds: ids,
				targetPath: destination || '/'
			})
		},
		onSuccess: (result) => {
			const queued =
				(result as { queueCrosspointDeliveries?: readonly unknown[] }).queueCrosspointDeliveries?.length ?? count
			toast.success(`${queued} ${queued === 1 ? 'book is' : 'books are'} confirmed in the local CrossPoint queue.`)
			void queryClient.invalidateQueries({ queryKey: ['crosspoint-deliveries'] })
		},
		onError: (error) => toast.error(errorMessage(error))
	}))

	function enqueue(targetId: string): void {
		if (!targetId || !ids.length || queue.isPending) return
		queue.mutate(targetId)
	}

	const label = $derived(count > 1 ? `Send ${count} to CrossPoint` : 'Send to CrossPoint')
</script>

{#if selectableTargets.length === 1}
	<Button
		size="xs"
		variant="ghost"
		title={`${label}; queues local work only`}
		onclick={() => enqueue(selectableTargets[0].id)}
		disabled={queue.isPending || !ids.length}
	>
		<SendIcon data-icon="inline-start" aria-hidden="true" />
		{queue.isPending ? 'Queuing…' : label}
	</Button>
{:else if selectableTargets.length > 1}
	<Select.Root type="single" onValueChange={enqueue}>
		<Select.Trigger
			class="h-7 gap-1 border-0 px-2 text-xs shadow-none"
			aria-label={label}
			disabled={queue.isPending || !ids.length}
		>
			<SendIcon class="size-3.5" aria-hidden="true" />
			{queue.isPending ? 'Queuing…' : label}
		</Select.Trigger>
		<Select.Content>
			<Select.Group>
				<Select.Label>Queue on device</Select.Label>
				{#each selectableTargets as target (target.id)}
					<Select.Item value={target.id} label={target.name} />
				{/each}
			</Select.Group>
		</Select.Content>
	</Select.Root>
{/if}
