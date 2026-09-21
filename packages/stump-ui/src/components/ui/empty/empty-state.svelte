<script lang="ts">
	/**
	 * The whole empty state in one tag: media, title, description, actions.
	 * The `Empty.*` parts stay available for layouts that need something
	 * else, but every "nothing here yet" panel in the apps is this shape, and
	 * repeating six tags per screen is how they drift apart.
	 */
	import type { Snippet } from "svelte";
	import type { HTMLAttributes } from "svelte/elements";
	import { cn, type WithoutChildren, type WithElementRef } from "@stump/ui/utils.js";
	import Content from "./empty-content.svelte";
	import Description from "./empty-description.svelte";
	import Header from "./empty-header.svelte";
	import Media from "./empty-media.svelte";
	import Title from "./empty-title.svelte";
	import Root from "./empty.svelte";

	let {
		ref = $bindable(null),
		class: className,
		title,
		description,
		icon,
		illustration,
		children,
		...restProps
	}: WithoutChildren<WithElementRef<HTMLAttributes<HTMLDivElement>>> & {
		title: string;
		description?: string;
		/** A lucide icon, boxed in the muted `icon` media slot. */
		icon?: Snippet;
		/** Anything richer than an icon; replaces the media slot wholesale. */
		illustration?: Snippet;
		/** The actions under the copy. */
		children?: Snippet;
	} = $props();
</script>

<Root bind:ref data-slot="empty-state" class={cn("rounded-xl border border-dashed bg-card p-10", className)} {...restProps}>
	<Header>
		{#if illustration}
			{@render illustration()}
		{:else if icon}
			<Media variant="icon">{@render icon()}</Media>
		{/if}
		<Title>{title}</Title>
		{#if description}
			<Description>{description}</Description>
		{/if}
	</Header>
	{#if children}
		<Content class="flex-row flex-wrap justify-center gap-2">{@render children()}</Content>
	{/if}
</Root>
