class Stack
  def initialize
    @items = []
  end

  def push(item)
    @items.push(item)
  end

  def pop
    raise IndexError, "stack is empty" if @items.empty?
    @items.pop
  end

  def each_item
    @items.each { |item| yield item }
  end
end
