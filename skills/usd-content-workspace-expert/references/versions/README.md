# Versions: what to pin before you answer

Every service in this stack ships on its own version line, and so does every API contract. Configuration keys, defaults, ports, chart schemas, and wire APIs change between versions. Establish which versions apply before giving configuration, commands, or client code, and say which version your answer is for.

## Four independent version axes

| Axis | What it identifies | Where it comes from | Read |
| --- | --- | --- | --- |
| **Running deployment** | The chart and image versions actually installed, and the API versions they advertise | The cluster: Helm release metadata, pod images, the Discovery catalog | [capture-a-deployment.md](capture-a-deployment.md) |
| **Released inventory** | Versions published together to NGC with a documentation release (currently usd-content-workspace Developer Guide 1.3.0) | NGC collection and published docs | [released-inventory.md](released-inventory.md) |
| **Source snapshot** | The component source in this repository at this commit | Each component's `CHANGELOG.md`, Cargo/pyproject/package versions, release commit subjects | [source-snapshot.md](source-snapshot.md) |
| **API contract** | Proto packages (`v1alpha`, `v1beta`, `v2alpha`), OpenAPI document versions, and API resource versions | `api/` directories, vendored proto `VERSION` pins, NGC API resources | [api-contracts.md](api-contracts.md) |

These axes do not move together. The source snapshot here is usually ahead of the released inventory: for example, Storage Service 1.2.3 is released, while this repo also carries unreleased Storage Service changes that turn URL normalization on by default. A chart version does not imply an image version, an image version does not imply an API version, and none of them is implied by this skill.

## Rules

1. **Running beats everything.** If the caller has a deployment, capture its versions first and answer for those. Use the matching component documentation, not the newest.
2. **Released for new installs.** For a new installation from NGC, use the released inventory unless the caller names another set. Do not mix artifacts from different inventories.
3. **Snapshot for source work.** For building, modifying, or testing code in this repo, use the snapshot versions and the files at this commit.
4. **Contracts match the server.** Generate clients from the API definitions the deployed service implements. A newer proto in this repo can add fields or packages the running service does not serve; the Discovery catalog shows which API versions a deployment advertises.
5. **Upgrades are migrations.** Changing a version string is not an upgrade. Read the installed-to-target changelog entries and migration guides; see [../operate/upgrades.md](../operate/upgrades.md).
6. **State the basis.** Every answer that depends on a version names it: "Storage Service 1.2.3 (released)", "event-consumer-service 1.12.0 (this repo's snapshot)", or "version unknown — capture it first".

## Version record

Keep one record per environment and update it on every change. Agents asking this expert should pass it along with their question.

```yaml
environment: <name>
captured: <date>
cluster: { distribution: <eks|aks|gke|microk8s|...>, kubernetes: <x.y> }
namespace: <namespace>
basis: <running | released:guide-1.3.0 | snapshot:<repo commit>>
components:
  - name: storage-service
    chart: <chart version>
    image: <image:tag or digest>
    apis: [<proto packages / REST versions advertised>]
  # one entry per installed component
contracts:
  storage-api: <resource or source version used to generate clients>
notes: <overrides, known deviations>
```
