# SYNTHESIZED — not real upstream bytes. `satysfi-deps.yaml` is never
# committed upstream (gitignored everywhere, saphe-split `demo/.gitignore:3`,
# `lib-satysfi/packages/.gitignore:4`), so this fixture is "honest
# synthesis": a 3-envelope subset of
# `demo/demo.saphe.lock.yaml.expected` (saphe-split @
# b836d512689248d18970674021ecaca409e0d897) transcribed through
# `make_deps_config`'s documented 1:1 mapping (`src-saphe/sapheMain.ml:635-640`
# — `locks` entries become `envelopes` entries; each `registered` payload
# resolves to an absolute `satysfi-envelope.yaml` path under the package
# store root). `{{ROOT}}` is a test-time placeholder for that store root.
#
# Real names (`registered.<registry-hash>.<pkg>.<version>`) and dependency
# shapes (`{name, used_as}`) are copied verbatim from the lock; only the
# store-relative `path` values are synthesized (the real store layout is a
# saphe implementation detail, out of scope here — Ld3b spec §3.5/§0.2).
envelopes:
- name: registered.6f2b80e9bb7c4e8af2104999fc25dbb3.stdlib.0.0.1
  path: "{{ROOT}}/store/stdlib.0.0.1/satysfi-envelope.yaml"
  dependencies: []
  test_only: false
- name: registered.6f2b80e9bb7c4e8af2104999fc25dbb3.math.0.0.1
  path: "{{ROOT}}/store/math.0.0.1/satysfi-envelope.yaml"
  dependencies:
  - name: registered.6f2b80e9bb7c4e8af2104999fc25dbb3.stdlib.0.0.1
    used_as: Stdlib
  test_only: false
- name: registered.6f2b80e9bb7c4e8af2104999fc25dbb3.tabular.0.0.1
  path: "{{ROOT}}/store/tabular.0.0.1/satysfi-envelope.yaml"
  dependencies:
  - name: registered.6f2b80e9bb7c4e8af2104999fc25dbb3.stdlib.0.0.1
    used_as: Stdlib
  test_only: false
dependencies:
- name: registered.6f2b80e9bb7c4e8af2104999fc25dbb3.stdlib.0.0.1
  used_as: Stdlib
- name: registered.6f2b80e9bb7c4e8af2104999fc25dbb3.math.0.0.1
  used_as: Math
- name: registered.6f2b80e9bb7c4e8af2104999fc25dbb3.tabular.0.0.1
  used_as: Tabular
test_dependencies: []
