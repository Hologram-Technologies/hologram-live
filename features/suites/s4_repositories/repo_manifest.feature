@status:specified
Feature: Canonical model repository manifest
  A model repository revision is one hologram.repo-manifest/v1 document. Its
  revision is the BLAKE3 kappa of its canonical bytes, and every file is
  reassembled from addressed chunks and verified before use.

  Not yet enforced: the runner drives the hologram binary, and no command or
  route exposes the manifest. Unit tests in src/repo_manifest.rs cover each
  scenario until the HF-compatible read surface lands (ADR 024).

  Scenario: identical content yields one revision
    Given a repository with files "config.json" and "model.safetensors"
    When I describe it twice, listing the files in different orders
    Then both manifests encode to identical bytes
    And both report the same revision

  Scenario: a large file is cut into fixed chunks
    Given a file of 10 bytes and a chunk size of 4 bytes
    When I describe the file
    Then it has chunks at offsets 0, 4 and 8 with sizes 4, 4 and 2

  Scenario: a manifest that escapes the repository is refused
    Given a manifest with a file path "../escape.bin"
    When I validate the manifest
    Then validation fails with a protocol error

  Scenario Outline: tampered content fails closed
    Given a described file stored as chunks
    When <defect>
    And I reassemble the file
    Then reassembly fails and returns no bytes

    Examples:
      | defect                                  |
      | one byte of a stored chunk is flipped   |
      | two chunk records are swapped           |
      | the whole-file blake3 is replaced       |
      | the declared sha256 is replaced         |
