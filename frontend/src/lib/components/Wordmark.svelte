<!-- The nib wordmark. Full riff on wide screens, collapses to `nib.` on narrow.
     Convention: halo-design's short, dry pop-culture riff ending in the app name. -->
<span class="halo-wordmark nib-wordmark">
  <span class="prefix">mightier than the prose. </span>nib<span class="accent">.</span>
</span>

<style>
  .nib-wordmark {
    font-size: 15px;
    user-select: none;
  }

  .prefix {
    color: var(--halo-text-muted);
  }

  @media (max-width: 720px) {
    .prefix {
      display: none;
    }
  }
</style>
