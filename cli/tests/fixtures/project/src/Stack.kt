package demo

class Stack<T> {
    private val items = mutableListOf<T>()

    val size: Int
        get() = items.size

    fun push(item: T) {
        items.add(item)
    }

    fun pop(): T {
        check(items.isNotEmpty()) { "stack is empty" }
        return items.removeAt(items.size - 1)
    }
}
