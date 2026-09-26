<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import * as Table from '@stump/ui/components/ui/table';
	import { request } from '@stump/ui/graphql/client';
	import { MyLoginActivityDocument } from '$lib/graphql/generated/graphql';
	import { getHomeSession } from '$lib/session.svelte';
	import { absoluteTime, relativeTime } from '$lib/format';

	const session = getHomeSession();
	const user = $derived(session.user);

	const activityQuery = createQuery(() => ({
		queryKey: ['loginActivity', user?.id],
		queryFn: () => request(MyLoginActivityDocument, { userId: user?.id ?? '' }),
		enabled: browser && !!user
	}));
	// Newest first, capped: the server returns the full history.
	const activity = $derived(
		[...(activityQuery.data?.loginActivityById ?? [])]
			.sort((a, b) => b.timestamp.localeCompare(a.timestamp))
			.slice(0, 20)
	);
</script>

<svelte:head>
	<title>Account · Coppice</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight">Account</h1>
		<p class="text-sm text-muted-foreground">Who you are on this server and where you signed in.</p>
	</div>

	<Card>
		<CardHeader>
			<CardTitle class="text-base">Profile</CardTitle>
		</CardHeader>
		<CardContent>
			{#if user}
				<dl class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-[10rem_1fr]">
					<dt class="text-muted-foreground">Username</dt>
					<dd class="flex items-center gap-2">
						{user.username}
						{#if user.isServerOwner}
							<Badge variant="secondary">Server owner</Badge>
						{/if}
					</dd>
					<dt class="text-muted-foreground">Single sign-on</dt>
					<dd>{user.oidcEmail ?? 'Not linked'}</dd>
					<dt class="text-muted-foreground">Last login</dt>
					<dd title={absoluteTime(user.lastLogin)}>{relativeTime(user.lastLogin)}</dd>
					<dt class="text-muted-foreground">Active sessions</dt>
					<dd>{user.loginSessionsCount}</dd>
				</dl>
			{:else}
				<Skeleton class="h-24" />
			{/if}
		</CardContent>
	</Card>

	<Card>
		<CardHeader>
			<CardTitle class="text-base">Recent sign-ins</CardTitle>
			<CardDescription>Password and single sign-on attempts against your account.</CardDescription>
		</CardHeader>
		<CardContent>
			{#if activityQuery.isPending}
				<Skeleton class="h-32" />
			{:else if activityQuery.isError}
				<Alert variant="destructive">
					<AlertTitle>Unable to load sign-in history</AlertTitle>
					<AlertDescription>
						{activityQuery.error instanceof Error ? activityQuery.error.message : 'Request failed.'}
					</AlertDescription>
				</Alert>
			{:else if activity.length === 0}
				<p class="text-sm text-muted-foreground">No sign-ins recorded yet.</p>
			{:else}
				<div class="overflow-hidden rounded-xl border">
					<Table.Root stacked class="table-fixed">
						<Table.Header>
							<Table.Row>
								<Table.Head class="w-32">When</Table.Head>
								<Table.Head class="w-44">From</Table.Head>
								<Table.Head>Client</Table.Head>
								<Table.Head class="w-20 text-right">Result</Table.Head>
							</Table.Row>
						</Table.Header>
						<Table.Body>
							{#each activity as entry (entry.id)}
								<Table.Row>
									<Table.Cell class="font-medium" title={absoluteTime(entry.timestamp)}>
										{relativeTime(entry.timestamp)}
									</Table.Cell>
									<Table.Cell data-label="From" class="whitespace-normal">
										<span class="font-mono text-xs break-all">{entry.ipAddress}</span>
									</Table.Cell>
									<Table.Cell data-label="Client" class="text-xs text-muted-foreground">
										<span class="block max-w-full truncate" title={entry.userAgent}>{entry.userAgent}</span>
									</Table.Cell>
									<Table.Cell data-label="Result" class="text-right">
										{#if entry.authenticationSuccessful}
											<Badge variant="outline">OK</Badge>
										{:else}
											<Badge variant="destructive">Failed</Badge>
										{/if}
									</Table.Cell>
								</Table.Row>
							{/each}
						</Table.Body>
					</Table.Root>
				</div>
			{/if}
		</CardContent>
	</Card>
</div>
