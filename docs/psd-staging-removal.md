# PSD export staging removal

Layer and mask export gathers big-endian channel planes directly from tiles. Each tile stays
pinned while its rows are read, and final planes use the bounded scratch allocator. Background
layers omit their unused transparency plane before gathering and compression. Extra merged
channels write into their final tail, reserved together to avoid allocation growth slack.

Raw merged output takes ownership of the completed planes with capacity limited to their
length. The native CMYK compositor writes into final plane slices without intermediate worker
planes, preserving its ink arithmetic and alpha rules. Other merged rendering stays banded.

Imports retain main's bounded row decoders, fallible interleaving and in-place mask conversion.
Native parallel execution and sequential WebAssembly execution are unchanged. Public conversion
APIs, Lab16 scaling, CMYK and spot-channel polarity, defaults and sample precision are preserved.

Equivalence tests cover every sample depth in RGB, Gray, CMYK and Lab, negative tile origins,
padding, mask defaults, Background channel omission, and raw merged planes with extra channels.
Release measurements use a fixed main baseline and identical saved documents and PSD/PSB bytes.
