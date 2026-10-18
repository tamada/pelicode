package demo;

import java.util.ArrayList;
import java.util.List;

public class Stack<T> {
    private final List<T> items = new ArrayList<>();

    public void push(T item) {
        items.add(item);
    }

    public T pop() {
        if (items.isEmpty()) {
            throw new IllegalStateException("stack is empty");
        }
        return items.remove(items.size() - 1);
    }
}
