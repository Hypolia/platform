variable "IMAGE_REGISTRY" {
  default = "ghcr.io/hypolia"
}

variable "IMAGE_TAG" {
  default = "dev"
}

variable "EXTRA_TAG" {
  default = ""
}

variable "SDK_DIR" {
  default = "../sdk"
}

function "image_tags" {
  params = [name]
  result = concat(
    ["${IMAGE_REGISTRY}/${name}:${IMAGE_TAG}"],
    EXTRA_TAG != "" ? ["${IMAGE_REGISTRY}/${name}:${EXTRA_TAG}"] : []
  )
}

group "default" {
  targets = ["orchestrator", "velocity"]
}

target "_common" {
  dockerfile = "Dockerfile"
  context    = "."
  contexts = {
    sdk-src = SDK_DIR
  }
  args = {
    HYPOLIA_VERSION = EXTRA_TAG != "" ? EXTRA_TAG : IMAGE_TAG
  }
}

target "orchestrator" {
  inherits = ["_common"]
  target   = "orchestrator"
  tags     = image_tags("orchestrator")
}

target "velocity" {
  inherits = ["_common"]
  target   = "velocity"
  tags     = image_tags("velocity")
}
