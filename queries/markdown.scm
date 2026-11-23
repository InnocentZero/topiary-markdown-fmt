; Based on topiary-queries 0.8.0, MIT, The Topiary authors.
[
  (paragraph)
  (html_block)
  (indented_code_block)
  (list)
  (thematic_break)
  (block_quote)
  (setext_heading)
  (atx_heading)
  (minus_metadata)
  (plus_metadata)
  (pipe_table)
  (link_reference_definition)
  (fenced_code_block)
] @leaf

; Markdown's leaves can own line terminators (and CRLF payload bytes).
; The adapter applies this directive only to the whitespace between blocks.
([
  (paragraph) (html_block) (indented_code_block) (list)
  (thematic_break) (block_quote) (setext_heading) (atx_heading)
  (minus_metadata) (plus_metadata) (pipe_table) (fenced_code_block)
  (link_reference_definition)
] @prepend_delimiter
 (#delimiter! "\n\n"))
