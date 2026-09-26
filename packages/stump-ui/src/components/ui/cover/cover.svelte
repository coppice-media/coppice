<script lang="ts">
	/**
	 * A cover image with a real fallback. While the URL is missing, or once
	 * the request fails, the tile shows a muted icon instead of the browser's
	 * broken-image glyph. The caller sizes the tile (`h-18 w-12`, `size-14`,
	 * `w-full`); `aspect` only matters when one dimension is left open.
	 */
	import BookOpenIcon from "@lucide/svelte/icons/book-open";
	import type { Snippet } from "svelte";
	import type { HTMLAttributes } from "svelte/elements";
	import { cn, type WithElementRef, type WithoutChildren } from "@stump/ui/utils.js";

	let {
		ref = $bindable(null),
		src,
		alt = "",
		aspect = "book",
		loading = "lazy",
		class: className,
		imgClass,
		fallback,
		...restProps
	}: WithoutChildren<WithElementRef<HTMLAttributes<HTMLDivElement>>> & {
		src?: string | null;
		/** Empty for decorative covers next to the title; a real description otherwise. */
		alt?: string;
		/** `book` is the 2:3 portrait most covers have; `square` fits audiobooks and avatars. */
		aspect?: "book" | "square" | "auto";
		loading?: "lazy" | "eager";
		/** Extra classes for the `<img>` itself (object position, transitions). */
		imgClass?: string;
		/** Replaces the default icon inside the fallback tile. */
		fallback?: Snippet;
	} = $props();

	// Remembering which URL failed (rather than a boolean) means a new `src`
	// gets a fresh attempt without an effect to reset the flag.
	let failedSrc = $state<string | null>(null);
	const showImage = $derived(Boolean(src) && failedSrc !== src);
</script>

<div
	bind:this={ref}
	data-slot="cover"
	data-state={showImage ? "image" : "fallback"}
	role={showImage || !alt ? undefined : "img"}
	aria-label={showImage || !alt ? undefined : alt}
	class={cn(
		"relative flex shrink-0 items-center justify-center overflow-hidden bg-muted text-muted-foreground",
		aspect === "book" && "aspect-[2/3]",
		aspect === "square" && "aspect-square",
		className
	)}
	{...restProps}
>
	{#if showImage}
		<img
			{src}
			{alt}
			{loading}
			decoding="async"
			class={cn("size-full object-cover", imgClass)}
			onerror={() => (failedSrc = src ?? null)}
		/>
	{:else if fallback}
		{@render fallback()}
	{:else}
		<BookOpenIcon aria-hidden="true" class="size-[clamp(1rem,35%,2.5rem)]" />
	{/if}
</div>
