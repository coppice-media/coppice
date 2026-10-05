import { describe, expect, test } from "bun:test";

import {
  appEndpointsForSource,
  attachKomeliaCallSites,
  extractKomfClientOperations,
  parseDtoTypes,
  parseSource,
  stableJson,
} from "./extract-komf-contract.mjs";

const CLIENT_PREFIX = "komf-client/src/commonMain/kotlin/snd/komf/client/";
const APP_PATH = "komf-app/src/main/kotlin/snd/komf/app/api/MangaBakaRoutes.kt";
const REF = "d8a34e9df29ddaa6941c302df216812ee6d525e9";
const parse = (path, text) => parseSource("fixture", REF, { path }, text);

const inlineRoutes = `
class MangaBakaRoutes(private val httpClient: Flow<HttpClient>) {
    fun registerRoutes(routing: Route) {
        routing.route("/mangabaka") {
            get("/gstatic-favicon") { getFavIcon() }
            route("/series") {
                get("/{seriesId}/cover") { getCover() }
                get("/tags") { getTags() }
                route("/linked") {
                    get("/{seriesId}") { getLinked() }
                    post("/batch") { batch() }
                }
                route("/link") {
                    post { link() }
                    delete { unlink() }
                    get("/search") { search() }
                    post("/match") { match() }
                }
            }
        }
    }
    private suspend fun RoutingContext.getFavIcon() {
        val response = httpClient.first().get("https://favicon.example.test/")
        call.respond(response.bodyAsBytes())
    }
    private suspend fun RoutingContext.getCover() { call.respond(HttpStatusCode.OK) }
    private suspend fun RoutingContext.getTags() { call.respond(emptyList<String>()) }
    private suspend fun RoutingContext.getLinked() { call.respond(HttpStatusCode.NotFound) }
    private suspend fun RoutingContext.batch() { call.receive<List<String>>() }
    private suspend fun RoutingContext.link() { call.receive<KomfMangaBakaLinkRequest>() }
    private suspend fun RoutingContext.unlink() { call.receive<KomfMangaBakaUnlinkRequest>() }
    private suspend fun RoutingContext.search() { call.respond(emptyList<String>()) }
    private suspend fun RoutingContext.match() { call.respond(HttpStatusCode.NotImplemented) }
}
`;

const mangaBakaClient = `
class KomfMangaBakaClient(private val ktor: HttpClient) {
    private val metadataApiPrefix = "api/mangabaka"
    suspend fun getLinked(id: KomfServerSeriesId): KomfMangaBakaLinkedSeries? {
        return ktor.get("$metadataApiPrefix/series/linked/$id").body()
    }
    suspend fun getAllLinked(ids: List<KomfServerSeriesId>): List<KomfMangaBakaLinkedSeries> {
        return ktor.post("$metadataApiPrefix/series/linked/batch") { setBody(ids) }.body()
    }
    suspend fun link(id: KomfServerSeriesId, mangaBakaId: MangaBakaSeriesId) {
        return ktor.post("$metadataApiPrefix/series/link") {
            setBody(KomfMangaBakaLinkRequest(id, mangaBakaId))
        }.body()
    }
    suspend fun unlink(id: KomfServerSeriesId) {
        return ktor.delete("$metadataApiPrefix/series/link") {
            setBody(KomfMangaBakaUnlinkRequest(id))
        }.body()
    }
    suspend fun match(id: KomfServerSeriesId) {
        return ktor.delete("$metadataApiPrefix/series/link/match") {
            setBody(KomfMangaBakaUnlinkRequest(id))
        }.body()
    }
    suspend fun getCover(id: MangaBakaSeriesId): ByteArray {
        return ktor.get("$metadataApiPrefix/series/$id/cover").body()
    }
}
`;

const bodyModels = parse("komf-api-models/src/commonMain/kotlin/snd/komf/api/mangabaka/Requests.kt", `
@Serializable
data class KomfMangaBakaLinkRequest(val komgaId: KomfServerSeriesId, val mangaBakaSeriesId: MangaBakaSeriesId)
@Serializable
data class KomfMangaBakaUnlinkRequest(val komgaId: KomfServerSeriesId)
`);

const clientOperations = () => extractKomfClientOperations([
  parse(`${CLIENT_PREFIX}KomfMangaBakaClient.kt`, mangaBakaClient),
  bodyModels,
  parse(`${CLIENT_PREFIX}KomfMetadataClient.kt`, `
class KomfMetadataClient(private val ktor: HttpClient) {
    suspend fun matchSeries(libraryId: KomfServerLibraryId, seriesId: KomfServerSeriesId): KomfMetadataJobResponse {
        return ktor.post("$metadataApiPrefix/match/library/$libraryId/series/$seriesId").body()
    }
}
`),
]);

const operation = (operations, name) => operations.find((item) => item.function === name);

describe("Komf 2.1 inline route inventory", () => {
  test("joins nested registrations and resolves handlers without treating outbound HTTP as a server route", () => {
    const endpoints = appEndpointsForSource(parse(APP_PATH, inlineRoutes));
    const routes = endpoints.filter((item) => item.verb !== "ROUTE");
    expect(routes.map((item) => `${item.verb} /api${item.registrationPrefix}${item.path}`)).toEqual([
      "GET /api/mangabaka/gstatic-favicon",
      "GET /api/mangabaka/series/{seriesId}/cover",
      "GET /api/mangabaka/series/tags",
      "GET /api/mangabaka/series/linked/{seriesId}",
      "POST /api/mangabaka/series/linked/batch",
      "POST /api/mangabaka/series/link",
      "DELETE /api/mangabaka/series/link",
      "GET /api/mangabaka/series/link/search",
      "POST /api/mangabaka/series/link/match",
    ]);
    expect(routes.map((item) => item.function)).toEqual([
      "getFavIcon", "getCover", "getTags", "getLinked", "batch", "link", "unlink", "search", "match",
    ]);
    expect(endpoints.filter((item) => item.verb === "ROUTE").map((item) => item.path)).toEqual([
      "/mangabaka", "/series", "/linked", "/link",
    ]);
    expect(routes.some((item) => item.path.includes("favicon.example.test"))).toBe(false);
  });

  test("fails closed on dynamic route paths, missing handlers, and ambiguous handler lambdas", () => {
    expect(() => appEndpointsForSource(parse(APP_PATH, inlineRoutes.replace('get("/tags")', "get(dynamicPath)"))))
      .toThrow();
    expect(() => appEndpointsForSource(parse(APP_PATH, inlineRoutes.replace("{ batch() }", "{ missingHandler() }"))))
      .toThrow();
    expect(() => appEndpointsForSource(parse(APP_PATH, inlineRoutes.replace("{ batch() }", "{ batch(); link() }"))))
      .toThrow();
    expect(() => appEndpointsForSource(parse(APP_PATH, inlineRoutes.slice(0, -3))))
      .toThrow();
  });

  test("keeps pre-existing non-inline route extraction unchanged", () => {
    const endpoints = appEndpointsForSource(parse("komf-app/src/main/kotlin/snd/komf/app/api/ConfigRoutes.kt", `
class ConfigRoutes {
    private fun Route.getConfigRoute() {
        get("/config") { call.respond(config) }
    }
}
`));
    expect(endpoints.map(({ verb, path, function: name }) => ({ verb, path, name })))
      .toEqual([{ verb: "GET", path: "/config", name: "getConfigRoute" }]);
  });

});

describe("Komelia 0.20 optional clients", () => {
  test("normalizes the MangaBaka prefix and extracts constructed request DTOs without hiding the DELETE match mismatch", () => {
    const operations = clientOperations();
    expect(operation(operations, "getAllLinked").route.path).toBe("/api/mangabaka/series/linked/batch");
    expect(operation(operations, "getLinked").route.path).toBe("/api/mangabaka/series/linked/{id}");
    expect(operation(operations, "link").request.body).toEqual({
      name: null, type: "KomfMangaBakaLinkRequest", expression: "KomfMangaBakaLinkRequest(id, mangaBakaId)",
    });
    expect(operation(operations, "unlink").request.body.type).toBe("KomfMangaBakaUnlinkRequest");
    expect(operation(operations, "getAllLinked").request.body).toEqual({ name: "ids", type: "List<KomfServerSeriesId>" });
    expect(operation(operations, "match").route.method).toBe("DELETE");
    expect(operation(operations, "match").optionalFeature).toBe("MangaBaka");
  });

  test("finds Flow.first browse calls, direct fetchers, secondary-constructor callbacks, and top-level calls", () => {
    const operations = clientOperations();
    attachKomeliaCallSites(operations, [parse("komelia-domain/Consumers.kt", `
class RemoteSeriesApi(private val komfMangaBakaClient: Flow<KomfMangaBakaClient?>) {
    suspend fun getOneSeries(id: KomfServerSeriesId) {
        komfMangaBakaClient.first()?.getLinked(id)
    }
    suspend fun toKomeliaPage(ids: List<KomfServerSeriesId>) {
        this.komfMangaBakaClient.first()?.getAllLinked(ids).orEmpty()
    }
}
class RemoteCollectionsApi(private val komfMangaBakaClient: Flow<KomfMangaBakaClient?>) {
    suspend fun getSeriesForCollection(ids: List<KomfServerSeriesId>) {
        komfMangaBakaClient.first()?.getAllLinked(ids).orEmpty()
    }
}
class KomfMangaBakaSeriesFetcher(private val komf: KomfMangaBakaClient, private val id: MangaBakaSeriesId) {
    suspend fun fetchBytes() = komf.getCover(id)
}
class Unrelated(private val other: OtherClient) {
    suspend fun load(ids: List<KomfServerSeriesId>) { other.getAllLinked(ids) }
}
data class SeriesBulkActions(val identify: suspend () -> Unit) {
    constructor(komfClient: KomfMetadataClient, libraryId: KomfServerLibraryId, seriesId: KomfServerSeriesId)
        : this(identify = { komfClient.matchSeries(libraryId, seriesId) })
}
fun bulkMatch(komfClient: KomfMetadataClient, libraryId: KomfServerLibraryId, seriesId: KomfServerSeriesId) {
    komfClient.matchSeries(libraryId, seriesId)
}
`)]);
    expect(operation(operations, "getLinked").callSites.map((site) => site.callerClass)).toEqual(["RemoteSeriesApi"]);
    expect(operation(operations, "getAllLinked").callSites.map((site) => site.callerClass))
      .toEqual(["RemoteSeriesApi", "RemoteCollectionsApi"]);
    expect(operation(operations, "getCover").callSites[0].callerClass).toBe("KomfMangaBakaSeriesFetcher");
    expect(operation(operations, "matchSeries").callSites).toHaveLength(2);
    expect(operation(operations, "matchSeries").callSites[0]).toMatchObject({ callerClass: "SeriesBulkActions", callerFunction: "constructor" });
    expect(operation(operations, "matchSeries").callSites[1]).toMatchObject({ callerClass: null, callerFunction: "bulkMatch" });
  });

  test("fails closed on a request body whose wire type cannot be resolved", () => {
    const malformed = mangaBakaClient.replace("setBody(ids)", "setBody(ids ?: fallback)");
    expect(() => extractKomfClientOperations([parse(`${CLIENT_PREFIX}KomfMangaBakaClient.kt`, malformed), bodyModels]))
      .toThrow();
  });

  test("inventories new value classes, required DTO fields, nullable fields, enums, and HeartbeatEvent", () => {
    const types = parseDtoTypes([parse("komf-api-models/Models.kt", `
@Serializable
value class MangaBakaSeriesId(val value: Long)
@Serializable
data class KomfMangaBakaLinkRequest(val komgaId: KomfServerSeriesId, val mangaBakaSeriesId: MangaBakaSeriesId)
@Serializable
data class MangaBakaCover(val x350: String?)
@Serializable
enum class MangaBakaSeriesState { ACTIVE, MERGED, DELETED }
@Serializable
sealed interface DownloadProgress {
    @Serializable
    @SerialName("HeartbeatEvent")
    data object HeartbeatEvent : DownloadProgress
}
`)]);
    expect(types.find((type) => type.name === "MangaBakaSeriesId").fields[0].type).toBe("Long");
    expect(types.find((type) => type.name === "KomfMangaBakaLinkRequest").fields.map((field) => field.required)).toEqual([true, true]);
    expect(types.find((type) => type.name === "MangaBakaCover").fields[0]).toMatchObject({ nullable: true, required: false });
    expect(types.find((type) => type.name === "MangaBakaSeriesState").enumValues).toEqual(["ACTIVE", "MERGED", "DELETED"]);
    expect(types.find((type) => type.name === "HeartbeatEvent")).toMatchObject({ kind: "dataObject", serialName: "HeartbeatEvent" });
  });
});

test("serialized contract ordering does not depend on object insertion order", () => {
  expect(stableJson({ z: [{ b: 2, a: 1 }], a: true })).toBe(stableJson({ a: true, z: [{ a: 1, b: 2 }] }));
});
