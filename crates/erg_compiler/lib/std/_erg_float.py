from _erg_result import Error
from _erg_control import then__

class Float(float):
    def try_new(i): # -> Result[Nat]
        if isinstance(i, float):
            Float(i)
        else:
            Error("not a float")
    def mutate(self):
        return FloatMut(self)
    def __add__(self, other):
        return then__(super().__add__(other), Float)
    def __radd__(self, other):
        return then__(super().__radd__(other), Float)
    def __sub__(self, other):
        return then__(super().__sub__(other), Float)
    def __rsub__(self, other):
        return then__(super().__rsub__(other), Float)
    def __mul__(self, other):
        return then__(super().__mul__(other), Float)
    def __rmul__(self, other):
        return then__(super().__rmul__(other), Float)
    def __div__(self, other):
        return then__(super().__div__(other), Float)
    def __rdiv__(self, other):
        return then__(super().__rdiv__(other), Float)
    def __floordiv__(self, other):
        return then__(super().__floordiv__(other), Float)
    def __rfloordiv__(self, other):
        return then__(super().__rfloordiv__(other), Float)
    def __pow__(self, other):
        return then__(super().__pow__(other), Float)
    def __rpow__(self, other):
        return then__(super().__rpow__(other), Float)

class FloatMut(): # inherits Float
    value: Float

    def __init__(self, i):
        self.value = Float(i)
    def __repr__(self):
        return self.value.__repr__()
    def __deref__(self):
        return self.value
    def __eq__(self, other):
        if isinstance(other, Float):
            return self.value == other
        else:
            return self.value == other.value
    def __ne__(self, other):
        if isinstance(other, Float):
            return self.value != other
        else:
            return self.value != other.value
    def __le__(self, other):
        if isinstance(other, Float):
            return self.value <= other
        else:
            return self.value <= other.value
    def __ge__(self, other):
        if isinstance(other, Float):
            return self.value >= other
        else:
            return self.value >= other.value
    def __lt__(self, other):
        if isinstance(other, Float):
            return self.value < other
        else:
            return self.value < other.value
    def __gt__(self, other):
        if isinstance(other, Float):
            return self.value > other
        else:
            return self.value > other.value
    def __add__(self, other):
        if isinstance(other, Float):
            return FloatMut(self.value + other)
        else:
            return FloatMut(self.value + other.value)
    def __sub__(self, other):
        if isinstance(other, Float):
            return FloatMut(self.value - other)
        else:
            return FloatMut(self.value - other.value)
    def __mul__(self, other):
        if isinstance(other, Float):
            return FloatMut(self.value * other)
        else:
            return FloatMut(self.value * other.value)
    def __floordiv__(self, other):
        if isinstance(other, Float):
            return FloatMut(self.value // other)
        else:
            return FloatMut(self.value // other.value)
    def __pow__(self, other):
        if isinstance(other, Float):
            return FloatMut(self.value ** other)
        else:
            return FloatMut(self.value ** other.value)
