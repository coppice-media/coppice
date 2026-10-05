<script lang="ts">
	/**
	 * Permissions grouped by area. `granted` holds only the ticked choices;
	 * whatever they imply on the server shows ticked but locked, naming the
	 * permission that includes it, so nobody unticks something the server
	 * would grant back on save.
	 */
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Checkbox } from '@stump/ui/components/ui/checkbox';
	import { Label } from '@stump/ui/components/ui/label';
	import type { UserPermission } from '$lib/graphql/generated/graphql';
	import {
		PERMISSION_GROUPS,
		PERMISSIONS,
		grantPermission,
		includedBy,
		withImplied
	} from '$lib/users';

	let {
		granted = $bindable([]),
		disabled = false
	}: { granted?: UserPermission[]; disabled?: boolean } = $props();

	const effective = $derived(withImplied(granted));

	function toggle(permission: UserPermission, checked: boolean): void {
		granted = checked
			? grantPermission(granted, permission)
			: granted.filter((entry) => entry !== permission);
	}
</script>

<div class="flex flex-col gap-5">
	{#each PERMISSION_GROUPS as group (group.key)}
		{@const grantedCount = group.permissions.filter((permission) => effective.has(permission)).length}
		<fieldset class="flex flex-col gap-1">
			<legend class="mb-1 flex w-full items-baseline justify-between gap-3 text-sm font-medium">
				{group.label}
				<span class="text-xs font-normal text-muted-foreground tabular-nums">
					{grantedCount} of {group.permissions.length}
				</span>
			</legend>
			{#each group.permissions as permission (permission)}
				{@const info = PERMISSIONS[permission]}
				{@const ticked = granted.includes(permission)}
				{@const sources = includedBy(granted, permission)}
				{@const included = !ticked && sources.length > 0}
				{@const id = `permission-${permission}`}
				<div class="flex items-start gap-3 rounded-md px-2 py-1.5 hover:bg-muted/40">
					<Checkbox
						{id}
						class="mt-0.5"
						checked={ticked || included}
						disabled={disabled || included}
						onCheckedChange={(checked) => toggle(permission, checked)}
						aria-describedby={`${id}-description`}
					/>
					<div class="flex min-w-0 flex-col gap-1">
						<Label for={id} class="flex-wrap font-normal">
							{info.label}
							{#if included}
								<Badge variant="outline">Included</Badge>
							{/if}
						</Label>
						<p id={`${id}-description`} class="text-xs text-muted-foreground">
							{info.description}
							{#if included}
								Included with {sources.map((source) => PERMISSIONS[source].label).join(', ')}.
							{/if}
						</p>
					</div>
				</div>
			{/each}
		</fieldset>
	{/each}
</div>
