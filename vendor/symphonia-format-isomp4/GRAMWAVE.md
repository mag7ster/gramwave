This directory contains Symphonia's published ISO/MP4 demuxer at version 0.6.1.
Gramwave changes the initial atom scan to seek past a known-size media payload
when the movie index is still missing. This lets a range source open a file
with a trailing index without reading the intervening audio. Drop the patch
when the upstream reader provides the same behavior.
