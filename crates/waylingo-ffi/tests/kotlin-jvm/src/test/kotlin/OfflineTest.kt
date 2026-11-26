import io.waylingo.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull
import kotlin.test.assertTrue

/** Checks that need no network: the library loads, and bad input is rejected. */
class OfflineTest {
    private val config = WaylingoConfig(
        mapProvider = MapProvider.TOMTOM,
        llm = LlmBackend.Anthropic(model = null, baseUrl = null),
    )

    @Test
    fun theLibraryLoads() {
        assertTrue(version().isNotEmpty())
    }

    @Test
    fun anEmptyMapKeyIsRejected() {
        assertFailsWith<WaylingoException.InvalidConfig> { Waylingo(config, "", "llm-key") }
    }

    @Test
    fun aMissingLlmKeyIsRejected() {
        assertFailsWith<WaylingoException.InvalidConfig> { Waylingo(config, "map-key") }
    }

    @Test
    fun aResultThatIsNotWaylingosIsRejected() {
        val app = Waylingo(config, "map-key", "llm-key")
        assertFailsWith<WaylingoException.IncompatibleResult> {
            app.rerouteBlocking("{}", Choices())
        }
    }

    @Test
    fun parseReportsAnUnreachableLlm() {
        val local = Waylingo(
            WaylingoConfig(
                mapProvider = MapProvider.TOMTOM,
                llm = LlmBackend.OpenAiCompatible("http://127.0.0.1:9/v1", "m", false),
            ),
            "map-key",
        )
        val out = local.parseBlocking(
            TripInput(query = "Drive to Utrecht", currentLocation = LatLng(52.37, 4.89)),
        )
        assertEquals(ParseStatus.FAILED, out.status)
        assertEquals(ErrorCode.LLM_UNREACHABLE, out.errorCode)
        assertNull(out.planJson)
    }
}
