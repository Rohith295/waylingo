// Live smoke test of the Swift bindings on macOS: plan, reroute, errors.
// Needs TOMTOM_API_KEY and LLM_API_KEY (Anthropic) in the environment. Prints only
// statuses, counts and ids, never place names. See examples/swift/run.sh.

import Foundation


@main
struct Smoke {
    static func main() async {
        let env = ProcessInfo.processInfo.environment
        guard let mapKey = env["TOMTOM_API_KEY"], let llmKey = env["LLM_API_KEY"] else {
            print("set TOMTOM_API_KEY and LLM_API_KEY")
            exit(2)
        }
        var failures = 0
        let config = WaylingoConfig(
            mapProvider: .tomtom,
            llm: .anthropic(model: nil, baseUrl: nil),
            settings: SearchSettings(topN: 3)
        )
        do {
            let app = try Waylingo(config: config, mapKey: mapKey, llmKey: llmKey)
            let trip = TripInput(
                query: "Drive to Utrecht, coffee halfway",
                currentLocation: LatLng(lat: 52.3702, lng: 4.8952),
                locale: "en-GB"
            )
            let out = try await app.plan(trip: trip)
            print("plan: status \(out.status), stops \(out.stops.count), llm \(out.llmCalls), map \(out.mapCalls)")
            if out.status == .failed { failures += 1 }

            if let stop = out.stops.first(where: { !$0.alternatives.isEmpty }) {
                let alt = stop.alternatives[0].providerPlaceId
                let choices = Choices(stops: [
                    StopChoice(
                        stopRequestId: stop.stopRequestId,
                        occurrence: stop.occurrence,
                        pick: .place(providerPlaceId: alt)
                    ),
                ])
                let again = try await app.reroute(previousGeojson: out.geojson, choices: choices)
                let chosen = again.stops.first { $0.stopRequestId == stop.stopRequestId }
                let ok = again.llmCalls == 0 && again.mapCalls == 1
                    && chosen?.pick.providerPlaceId == alt && chosen?.userChoice == .chosen
                print("reroute: status \(again.status), llm \(again.llmCalls), map \(again.mapCalls), ok \(ok)")
                if !ok { failures += 1 }
            } else {
                print("no alternatives; reroute not exercised")
            }
        } catch {
            print("unexpected error: \(error)")
            failures += 1
        }

        do {
            _ = try Waylingo(config: config, mapKey: "", llmKey: llmKey)
            print("empty map key accepted: WRONG")
            failures += 1
        } catch WaylingoError.InvalidConfig {
            print("empty map key rejected: InvalidConfig")
        } catch {
            print("wrong error: \(error)")
            failures += 1
        }
        print("FAILURES: \(failures)")
        exit(failures == 0 ? 0 : 1)
    }
}
