class Stack:
    """A simple stack."""

    def __init__(self):
        self.items = []

    def push(self, item):
        self.items.append(item)

    def pop(self):
        """Removes and returns the top item."""
        if not self.items:
            raise IndexError("stack is empty")
        return self.items.pop()
