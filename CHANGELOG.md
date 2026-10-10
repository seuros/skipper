# Changelog

## [0.8.0](https://github.com/seuros/skipper/compare/skipper-v0.7.1...skipper-v0.8.0) (2026-10-10)


### Features

* blame each line to the commit that last changed it ([2edbbd6](https://github.com/seuros/skipper/commit/2edbbd615b3b9b103b3aebc2cb8cf1ddcf67a167))
* sign commits when commit.gpgsign is set ([f9dfa94](https://github.com/seuros/skipper/commit/f9dfa94df7c1ef2750ca3764194be726a7a8cb3b))


### Bug Fixes

* bound git_log's buffer, version parsing past non-ASCII, structured repo_search ([445cdab](https://github.com/seuros/skipper/commit/445cdabdf00781d72d95ace1c7c14f0d66033d46))
* honour [providers.&lt;forge&gt;] disabled; drop config skipper never read ([911f651](https://github.com/seuros/skipper/commit/911f651d106f5458f2a91e806ec277ce8b725449))


### Performance Improvements

* answer visibility predicates with has_custom ([cce6bef](https://github.com/seuros/skipper/commit/cce6befa645df0dd83766432578d77b9499512e8))
* resolve the CI target once per build watch ([cc80cb6](https://github.com/seuros/skipper/commit/cc80cb6a6226638db798640a4c2ba1eb0d7340df))
* stop copying what can be moved, borrowed or shared ([80df6ed](https://github.com/seuros/skipper/commit/80df6edfe3e664dddba72f22ee89e6c1be83794d))

## [0.7.1](https://github.com/seuros/skipper/compare/skipper-v0.7.0...skipper-v0.7.1) (2026-10-04)


### Bug Fixes

* let pr_merge wait past the call timeout for approval ([23d5432](https://github.com/seuros/skipper/commit/23d5432f3e16f3fa1eb32c2fe88b52f167a79d8c))
* retry git fetch, pull and push on network failures ([cc57ad0](https://github.com/seuros/skipper/commit/cc57ad06aca120942d155b1e6aaae4e1559f1117))

## [0.7.0](https://github.com/seuros/skipper/compare/skipper-v0.6.0...skipper-v0.7.0) (2026-10-04)


### Features

* lean PR reads: a merge-decision overview, description and files apart ([5110eb7](https://github.com/seuros/skipper/commit/5110eb7f10eef805ad40b87ef23e6c521a98c829))
* strip CodeRabbit noise from comments ([4282c03](https://github.com/seuros/skipper/commit/4282c03237d4aa0382f7fdabbc20b6745f420246))


### Bug Fixes

* keep collapsed &lt;details&gt; content in comment bodies ([c6a1c8b](https://github.com/seuros/skipper/commit/c6a1c8bc5b0bff31fd9973a3f4a847f48633beba))
* let the caller choose the repo for PR and issue reads; name forks in errors ([d02161d](https://github.com/seuros/skipper/commit/d02161d591252bebcc1c4ccc17a1eb9a9c772887))

## [0.6.0](https://github.com/seuros/skipper/compare/skipper-v0.5.0...skipper-v0.6.0) (2026-10-04)


### Features

* GitHub over REST/GraphQL with gh's token; adopt mcp-host 0.7 ([eb36561](https://github.com/seuros/skipper/commit/eb365613fa3d8158319cd00b58e7d0ee06a4947d))
* opt-in write tools: pr_merge, git_push, git_pull, git_fetch ([0120057](https://github.com/seuros/skipper/commit/012005744410e93a71a12835a574c7d4b26e7f85))
* PR overview resource, build_watch commit= ([1d4dd86](https://github.com/seuros/skipper/commit/1d4dd8604ac9bf62f956b21540fbc8bc952dc8ca))


### Bug Fixes

* full issue and PR bodies, issue triage fields, name issue resources ([ad18e36](https://github.com/seuros/skipper/commit/ad18e36024c7a2d3741177b70faf3bba8611cede))

## [0.5.0](https://github.com/seuros/skipper/compare/skipper-v0.4.0...skipper-v0.5.0) (2026-10-03)


### Features

* issue resources, read from the current remote ([9870502](https://github.com/seuros/skipper/commit/987050245337181b76366a18245e6d0597a974e8))
* resource read errors reach claude-code models ([e272e46](https://github.com/seuros/skipper/commit/e272e467f44f961ceab87ed07e26e0667686d566))


### Bug Fixes

* compact PR check output, ride out transient poll failures ([955a85c](https://github.com/seuros/skipper/commit/955a85c1ce43ca47ff214e2dee1ff42ba7471c4d))
* exclude nested test dirs from crate package ([fdcc463](https://github.com/seuros/skipper/commit/fdcc463842ceb446380905bd9e4def22df460544))
* **mcp:** emit compact JSON tool output ([cd424dc](https://github.com/seuros/skipper/commit/cd424dccba50cb4cf39001cc8d32d85b79c64803))

## [0.4.0](https://github.com/seuros/skipper/compare/skipper-v0.3.0...skipper-v0.4.0) (2026-09-29)


### Features

* PR discussion and PR list resources ([a316110](https://github.com/seuros/skipper/commit/a316110dd3f6622fe383b9329ac72633813a4ba5))

## [0.3.0](https://github.com/seuros/skipper/compare/skipper-v0.2.0...skipper-v0.3.0) (2026-09-29)


### Features

* per-client instructions, resource catalog for claude-code ([b00bfd9](https://github.com/seuros/skipper/commit/b00bfd974246758c00fea2c7a37db71139ac200b))
* pr_watch tool and skipper://watch resource ([19c0abf](https://github.com/seuros/skipper/commit/19c0abf8a728ea67183280e8cdfcae3d046792b2))
* release workflow, install script, and binary downloads ([376b97e](https://github.com/seuros/skipper/commit/376b97e0a3f2bf9a81d4979310f45592d28852f0))
* skipper://workspace resource ([abb8f46](https://github.com/seuros/skipper/commit/abb8f466d6101e7ae9cff6357b3b28cb12d64eb2))


### Bug Fixes

* **ci:** install rustfmt and clippy with the mise rust toolchain ([b1c5ac5](https://github.com/seuros/skipper/commit/b1c5ac567455ff2a72d3624a58299a53351778ec))
* retry forge auth probes that fail on the network ([759cf07](https://github.com/seuros/skipper/commit/759cf07a736a4dcafe47b816dca343ad1eb0bdf3))
