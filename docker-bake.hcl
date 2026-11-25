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

variable "RSPIGOT_DIR" {
  default = "/opt/minecraft/dev/rspigot"
}

variable "RSPIGOT_REF" {
  default = "local"
}

function "image_tags" {
  params = [name]
  result = concat(
    ["${IMAGE_REGISTRY}/${name}:${IMAGE_TAG}"],
    EXTRA_TAG != "" ? ["${IMAGE_REGISTRY}/${name}:${EXTRA_TAG}"] : []
  )
}

group "default" {
  targets = ["orchestrator", "proxy", "proxy-plugin"]
}

target "_common" {
  dockerfile = "Dockerfile"
  context    = "."
  contexts = {
    sdk-src     = SDK_DIR
    rspigot-src = RSPIGOT_DIR
  }
  args = {
    HYPOLIA_VERSION = EXTRA_TAG != "" ? EXTRA_TAG : IMAGE_TAG
    RSPIGOT_REF     = RSPIGOT_REF
  }
}

target "orchestrator" {
  inherits = ["_common"]
  target   = "orchestrator"
  tags     = image_tags("orchestrator")
}

target "proxy" {
  inherits = ["_common"]
  target   = "proxy"
  tags     = image_tags("proxy")
}

target "proxy-plugin" {
  inherits = ["_common"]
  target   = "proxy-plugin"
  tags     = image_tags("proxy-plugin")
}
