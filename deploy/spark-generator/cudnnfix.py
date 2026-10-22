import torch
from torch import nn


def _wrap(cls):
    orig = cls._conv_forward

    def _conv_forward(self, input, weight, bias):
        if input.is_cuda and input.dtype in (torch.float16, torch.bfloat16):
            with torch.backends.cudnn.flags(enabled=False):
                return orig(self, input, weight, bias)
        return orig(self, input, weight, bias)

    cls._conv_forward = _conv_forward


for _c in (nn.Conv1d, nn.Conv2d, nn.Conv3d):
    _wrap(_c)
